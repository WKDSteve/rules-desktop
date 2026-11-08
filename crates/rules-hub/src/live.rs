//! The routes that follow the vault: the event stream, the search and the request for a rescan.

use std::convert::Infallible;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

use appkit_hub::search::{self, Query, Status};
use appkit_hub::{bearer, error_body, json_response, no_query};
use axum::body::{Body, Bytes};
use axum::extract::{RawQuery, State};
use axum::http::header::{self, HeaderMap, HeaderValue};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use futures_core::Stream;
use serde_json::{json, Value};
use tokio::sync::{broadcast, mpsc};

use crate::answer::{engine_failure, engine_unavailable};
use crate::routes::Shared;
use crate::store::SHAPE;

/// The body of an event stream: what the task of the connection sends.
struct EventStream {
    rx: mpsc::Receiver<Bytes>,
}

impl Stream for EventStream {
    type Item = Result<Bytes, Infallible>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.rx.poll_recv(cx).map(|item| item.map(Ok))
    }
}

/// `GET /api/rules/events`: server-sent events. A `hello` first (with where the stream stands),
/// then one `rules` event per change. An event is a hint: what changed, as ids; the client reads
/// again. There is no replay: a client that connects reads the snapshot first. A client that was
/// too slow for the buffer gets `resync` and reads everything again.
pub(crate) async fn events(
    State(st): State<Arc<Shared>>,
    RawQuery(query): RawQuery,
    headers: HeaderMap,
) -> Response {
    if let Some(response) = no_query(query.as_deref()) {
        return response;
    }
    let rt = st.rt();
    if !rt.store.has_engine() {
        return engine_unavailable(&rt);
    }
    // The guard has checked the session; the token is the key of the count.
    let token = bearer(&headers).unwrap_or_default();
    let Some(guard) = rt
        .streams
        .acquire(&token, st.limits.max_streams_per_session)
    else {
        return error_body(
            StatusCode::TOO_MANY_REQUESTS,
            "too_many_streams",
            "this session has too many open event streams",
        );
    };
    // Start from a known state: the snapshot is held before the stream is.
    if let Err(error) = rt.store.current().await {
        return engine_failure(error);
    }
    let (mut changes, seq, rev) = rt.store.subscribe();
    let (tx, rx) = mpsc::channel::<Bytes>(16);
    let heartbeat = st.limits.heartbeat;
    let instance = st.instance.clone();
    let shared = Arc::clone(&st);
    tokio::spawn(async move {
        let _guard = guard;
        let hello = format!(
            "event: hello\ndata: {}\n\n",
            json!({ "rev": rev, "seq": seq, "instance": instance })
        );
        // The hello too: the session may have been closed while the snapshot was read.
        if !shared.sessions.check(&token) {
            return;
        }
        if tx.send(Bytes::from(hello)).await.is_err() {
            return;
        }
        let mut ticker =
            tokio::time::interval_at(tokio::time::Instant::now() + heartbeat, heartbeat);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            let message = tokio::select! {
                change = changes.recv() => match change {
                    Ok(c) => c.to_sse(SHAPE.event),
                    Err(broadcast::error::RecvError::Lagged(_)) => {
                        "event: resync\ndata: {}\n\n".to_owned()
                    }
                    Err(broadcast::error::RecvError::Closed) => return,
                },
                _ = ticker.tick() => ": ping\n\n".to_owned(),
                () = tx.closed() => return,
            };
            // A session that was closed since the stream was opened takes its streams with it: no
            // id goes to a token that is no longer one.
            if !shared.sessions.check(&token) {
                return;
            }
            if tx.send(Bytes::from(message)).await.is_err() {
                return;
            }
        }
    });
    let mut response = Response::new(Body::from_stream(EventStream { rx }));
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/event-stream; charset=utf-8"),
    );
    response
}

/// `GET /api/rules/search?q=<escaped text>&limit=<1..50>`: the rules whose title or text holds the
/// words. The text is the hub's own copy (the index of the kit), so the answer takes milliseconds.
/// While the index is built it says `building`; the page asks again.
pub(crate) async fn search(State(st): State<Arc<Shared>>, RawQuery(query): RawQuery) -> Response {
    let rt = st.rt();
    let Some(index) = &rt.search else {
        return engine_unavailable(&rt);
    };
    let (text, limit) = match search::parse_request(query.as_deref()) {
        Ok(parsed) => parsed,
        Err(why) => return error_body(StatusCode::BAD_REQUEST, "bad_query", why),
    };
    let query = match Query::parse(&text) {
        Ok(q) => q,
        Err(why) => return error_body(StatusCode::BAD_REQUEST, "bad_query", why),
    };
    let (status, rules, partial, outcome) = match index.status() {
        Status::Ready { items, partial } => ("ready", items, partial, index.search(&query, limit)),
        Status::Building => ("building", 0, false, None),
        Status::Failed => ("failed", 0, false, None),
    };
    let hits: Vec<Value> = outcome
        .as_ref()
        .map(|o| {
            o.hits
                .iter()
                .map(|h| {
                    json!({
                        "id": h.id,
                        "score": h.score,
                        "snippet": {
                            "text": h.snippet.text,
                            "marks": h.snippet.marks,
                            "more_before": h.snippet.more_before,
                            "more_after": h.snippet.more_after,
                        },
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    let body = json!({
        "schema": 1,
        "kind": "hub.search",
        "status": status,
        "indexed": rules,
        "partial": partial,
        "total": outcome.as_ref().map_or(0, |o| o.total),
        "hits": hits,
    });
    json_response(StatusCode::OK, body.to_string(), None)
}

/// `POST /api/rules/rescan`: a client that got the focus back asks for a scan. The answer is 202:
/// the scan runs on its own, and the event stream tells what it found.
pub(crate) async fn rescan(State(st): State<Arc<Shared>>) -> Response {
    let rt = st.rt();
    if !rt.store.has_engine() {
        return engine_unavailable(&rt);
    }
    rt.store.request_rescan();
    (StatusCode::ACCEPTED, axum::Json(json!({ "ok": true }))).into_response()
}
