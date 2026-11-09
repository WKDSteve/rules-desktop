//! The routes of the app, under `/api/rules/`. Each is under `/api/`, so the guard asked for a
//! session before a handler runs. The documents of the engine are passed on as the engine wrote
//! them.
//!
//! [`ROUTES`] is the one list of them: the router is built from it and the security matrix reads
//! its names, so a route cannot be served without the matrix trying it without a session.

use std::sync::Arc;

use appkit_hub::{
    error_body, etag_matches, json_response, no_query, not_modified, single, vault_request,
    vault_text, Core,
};
use axum::extract::{RawQuery, State};
use axum::http::header::{self, HeaderMap};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post, MethodRouter};
use axum::Router;
use serde_json::{json, Value};

use crate::answer::{ask, engine_failure, engine_unavailable};
use crate::engine::{Calls, EngineError};
use crate::live;
use crate::runtime::RulesRuntime;
use crate::IDENTITY;

/// The core of the hub with the runtime of this app.
pub type Shared = Core<RulesRuntime>;

/// How one route is answered: its method and handler.
pub type MakeRoute = fn() -> MethodRouter<Arc<Shared>>;

/// Every route of the app, by its name below `/api/rules/`.
pub const ROUTES: &[(&str, MakeRoute)] = &[
    ("snapshot", || get(snapshot)),
    ("rule", || get(rule)),
    ("sources", || get(sources)),
    ("about", || get(about)),
    ("events", || get(live::events)),
    ("search", || get(live::search)),
    ("rescan", || post(live::rescan)),
    ("open", || post(open)),
    ("autostart", || get(autostart_get).post(autostart_set)),
    ("next_id", || get(crate::write::next_id)),
    ("ops", || post(crate::write::ops)),
    ("waivers", || get(crate::write::waivers)),
    ("rulesets", || get(crate::write::rulesets)),
    ("groupings", || get(crate::write::groupings)),
    ("views", || get(crate::write::views)),
    ("sources/class", || post(crate::policy::set_class)),
];

/// The router of the app, built from [`ROUTES`] and from nothing else. Invariant: this stays a
/// pure fold over the table, with no extra `route` or `merge`, because [`crate::DATA_ROUTES`] and
/// the security matrix only know the routes of the table; a route added beside it would be served
/// without being tried there.
pub(crate) fn app_routes(_core: &Arc<Shared>) -> Router<Arc<Shared>> {
    ROUTES.iter().fold(Router::new(), |router, (name, make)| {
        router.route(&IDENTITY.route(name), make())
    })
}

/// `GET /api/rules/snapshot`: the held snapshot as the engine wrote it, with its digest as the
/// `ETag`; `If-None-Match` with that tag is a 304 without a body.
async fn snapshot(
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
    let cached = match rt.store.current().await {
        Ok(c) => c,
        Err(error) => return engine_failure(error),
    };
    if let Some(etag) = &cached.etag {
        let seen = single(&headers, &header::IF_NONE_MATCH).ok().flatten();
        if seen.is_some_and(|h| etag_matches(&h, etag)) {
            return not_modified(etag);
        }
    }
    json_response(StatusCode::OK, cached.text.clone(), cached.etag.as_deref())
}

/// The longest id the engine gives a rule, in bytes (rules.nvim docs/CONTRACT.md, `next_id`).
pub const MAX_ID_BYTES: usize = 100;

/// The id of `rule?id=<id>`. The value is percent-decoded (`+` is a space, as a form writes it)
/// and must be 1 to 100 bytes of UTF-8 without a control character. There is no pattern beyond
/// that: a rule in a document may have an id a pattern would refuse, and whether the id is one is
/// told by the snapshot, not by its shape.
pub(crate) fn rule_id(query: Option<&str>) -> Result<String, &'static str> {
    let query = query.filter(|q| !q.is_empty()).ok_or("id is missing")?;
    let mut id = None;
    for pair in query.split('&') {
        let (key, value) = pair.split_once('=').ok_or("a query part has no value")?;
        if key != "id" {
            return Err("unknown query parameter");
        }
        if id.replace(value).is_some() {
            return Err("a query parameter is given twice");
        }
    }
    let raw = id.ok_or("id is missing")?;
    // Longer than three times the limit cannot decode to an id: refused before it is decoded.
    if raw.len() > 3 * MAX_ID_BYTES {
        return Err("id is too long");
    }
    let decoded = percent_decode(raw).ok_or("id is not escaped right")?;
    let id = String::from_utf8(decoded).map_err(|_| "id is not UTF-8")?;
    if id.is_empty() || id.len() > MAX_ID_BYTES {
        return Err("id is 1 to 100 bytes");
    }
    if id.chars().any(char::is_control) {
        return Err("id holds a control character");
    }
    Ok(id)
}

fn percent_decode(raw: &str) -> Option<Vec<u8>> {
    let bytes = raw.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'%' => {
                let hex = bytes.get(i + 1..i + 3)?;
                let hex = std::str::from_utf8(hex).ok()?;
                out.push(u8::from_str_radix(hex, 16).ok()?);
                i += 3;
            }
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            b => {
                out.push(b);
                i += 1;
            }
        }
    }
    Some(out)
}

/// `GET /api/rules/rule?id=<id>`: the `rule` document of the engine, unchanged. Only an id of the
/// held snapshot is asked for; any other is a 404 without a question to the engine, so no string
/// from outside reaches it.
async fn rule(State(st): State<Arc<Shared>>, RawQuery(query): RawQuery) -> Response {
    let id = match rule_id(query.as_deref()) {
        Ok(id) => id,
        Err(why) => return error_body(StatusCode::BAD_REQUEST, "bad_query", why),
    };
    let rt = st.rt();
    if !rt.store.has_engine() {
        return engine_unavailable(&rt);
    }
    let cached = match rt.store.current().await {
        Ok(c) => c,
        Err(error) => return engine_failure(error),
    };
    let Some(held) = rt.held(&cached) else {
        return no_snapshot();
    };
    if !held.ids.contains(&id) {
        return error_body(StatusCode::NOT_FOUND, "not_found", "no rule with this id");
    }
    match ask(&rt, Calls::Rule, Some(&json!({ "id": id }))).await {
        Ok(reply) => json_response(StatusCode::OK, reply.text, None),
        Err(response) => response,
    }
}

/// `GET /api/rules/autostart`: whether the hub starts at login (`{ "on": true | false }`); `{ "on": null }` where this
/// system cannot tell or this hub has no such entry: the page then offers no switch.
async fn autostart_get(State(st): State<Arc<Shared>>) -> Response {
    let Some(login) = st.rt().login.clone() else {
        return axum::Json(serde_json::json!({ "on": null })).into_response();
    };
    match tokio::task::spawn_blocking(move || login.is_on()).await {
        Ok(on) => axum::Json(serde_json::json!({ "on": on })).into_response(),
        Err(_) => axum::Json(serde_json::json!({ "on": null })).into_response(),
    }
}

/// `POST /api/rules/autostart` `{ "on": bool }`: register the hub at login, or take it away. The answer is what the system
/// says afterwards, not what was asked.
async fn autostart_set(State(st): State<Arc<Shared>>, body: axum::body::Bytes) -> Response {
    let on = match serde_json::from_slice::<serde_json::Value>(&body) {
        Ok(doc) => match (
            doc.as_object().map(serde_json::Map::len),
            doc["on"].as_bool(),
        ) {
            (Some(1), Some(on)) => on,
            _ => {
                return error_body(
                    StatusCode::BAD_REQUEST,
                    "bad_request",
                    "the body is { \"on\": true | false }",
                )
            }
        },
        Err(_) => {
            return error_body(
                StatusCode::BAD_REQUEST,
                "bad_request",
                "the body is not JSON",
            )
        }
    };
    let Some(login) = st.rt().login.clone() else {
        return error_body(
            StatusCode::NOT_FOUND,
            "autostart_unavailable",
            "this hub has no login entry",
        );
    };
    let done = tokio::task::spawn_blocking(move || login.set(on).map(|()| login.is_on())).await;
    match done {
        Ok(Ok(now)) => axum::Json(serde_json::json!({ "on": now })).into_response(),
        Ok(Err(why)) => error_body(StatusCode::BAD_GATEWAY, "autostart_failed", &why),
        Err(_) => error_body(
            StatusCode::INTERNAL_SERVER_ERROR,
            "autostart_failed",
            "the change stopped unexpectedly",
        ),
    }
}

/// `POST /api/rules/open` with `{"id": "<rule id>"}`: show the file of the rule in Neovim, at its line. The
/// file is not named by the client and not worked out from the `source` of the rule (a name, not a place): the
/// engine says where it is and which loaded rule set holds it, and the file must be a plain file inside that rule
/// set once links are resolved. Only an id of the held snapshot is asked for.
async fn open(State(st): State<Arc<Shared>>, body: axum::body::Bytes) -> Response {
    let rt = st.rt();
    let Some(editor) = &rt.editor else {
        return error_body(
            StatusCode::NOT_FOUND,
            "open_unavailable",
            "this hub cannot open a file in an editor",
        );
    };
    let map = match vault_request(&body, &["id"]) {
        Ok(m) => m,
        Err(r) => return r,
    };
    let id = match vault_text(&map, "id", MAX_ID_BYTES) {
        Ok(id) if !id.chars().any(char::is_control) => id,
        Ok(_) => {
            return error_body(
                StatusCode::BAD_REQUEST,
                "bad_id",
                "id holds a control character",
            )
        }
        Err(r) => return r,
    };
    if !rt.store.has_engine() {
        return engine_unavailable(&rt);
    }
    let cached = match rt.store.current().await {
        Ok(c) => c,
        Err(error) => return engine_failure(error),
    };
    let Some(held) = rt.held(&cached) else {
        return no_snapshot();
    };
    if !held.ids.contains(&id) {
        return error_body(StatusCode::NOT_FOUND, "not_found", "no rule with this id");
    }
    let reply = match ask(&rt, Calls::Rule, Some(&json!({ "id": id }))).await {
        Ok(r) => r,
        Err(response) => return response,
    };
    let doc: Value = serde_json::from_str(&reply.text).unwrap_or(Value::Null);
    let line = doc["rule"]["line"]
        .as_u64()
        .and_then(|n| u32::try_from(n).ok())
        .filter(|n| *n > 0);
    // The place is the engine's to say: the `source` of a document is a name, not a place.
    let located = match editor.locator.locate(&id).await {
        Ok(l) => l,
        Err(why) => return error_body(StatusCode::BAD_GATEWAY, "engine_failed", why),
    };
    let Some(root) = &located.root else {
        return error_body(
            StatusCode::BAD_REQUEST,
            "open_refused",
            "the file is in none of the loaded rule sets",
        );
    };
    let file = match crate::editor::file_in_ruleset(root, &located.file) {
        Ok(f) => f,
        Err(why) => return error_body(StatusCode::BAD_REQUEST, "open_refused", why),
    };
    match editor.opener.open(&file, line) {
        Ok(()) => axum::Json(json!({ "ok": true })).into_response(),
        Err(why) => error_body(StatusCode::BAD_GATEWAY, "open_failed", &why),
    }
}

/// What a route that reads the held snapshot answers when it is not one.
pub(crate) fn no_snapshot() -> Response {
    eprintln!("engine: the snapshot has no list of rules or of files");
    error_body(
        StatusCode::BAD_GATEWAY,
        "engine_failed",
        "the engine could not answer",
    )
}

/// `GET /api/rules/sources`: every file of the held snapshot with the class it is shown with
/// (`sources.json` of this device), computed here and asked of no engine. The page takes the class
/// of a rule from this list by its `source` and works out none itself.
async fn sources(State(st): State<Arc<Shared>>, RawQuery(query): RawQuery) -> Response {
    if let Some(response) = no_query(query.as_deref()) {
        return response;
    }
    let rt = st.rt();
    if !rt.store.has_engine() {
        return engine_unavailable(&rt);
    }
    let cached = match rt.store.current().await {
        Ok(c) => c,
        Err(error) => return engine_failure(error),
    };
    // A snapshot without its files is not answered with an empty list: that would read as
    // "nothing to classify" instead of "not known".
    // Computed once for each snapshot the hub holds.
    let Some(body) = rt.held(&cached).and_then(|h| rt.sources_body(&h)) else {
        return no_snapshot();
    };
    json_response(StatusCode::OK, body, None)
}

/// `GET /api/rules/about`: what this build is and which engine it runs. Without an engine, or with
/// one that does not answer, the app is still told.
async fn about(State(st): State<Arc<Shared>>, RawQuery(query): RawQuery) -> Response {
    if let Some(response) = no_query(query.as_deref()) {
        return response;
    }
    let rt = st.rt();
    // An engine that is there but does not answer is told apart from no engine.
    let mut problem: Option<&'static str> = None;
    let engine = match &rt.engine {
        None => None,
        Some(engine) => match engine.call(Calls::Hello, None).await {
            Ok(reply) => match serde_json::from_str::<Value>(&reply.text) {
                // A `hello` without an `engine` object is an engine that broke the contract, not
                // a missing engine.
                Ok(doc) if doc["engine"].is_object() => Some(doc["engine"].clone()),
                Ok(_) => {
                    problem = Some("engine_protocol");
                    None
                }
                Err(_) => {
                    problem = Some("engine_failed");
                    None
                }
            },
            Err(error) => {
                problem = Some(about_problem(&error));
                None
            }
        },
    };
    let body = about_document(engine.as_ref(), problem);
    json_response(StatusCode::OK, body.to_string(), None)
}

/// A short, fixed word for why the engine did not answer `hello` (no text of the engine, no path).
fn about_problem(error: &EngineError) -> &'static str {
    match error {
        EngineError::Refused(_) => "engine_refused",
        EngineError::BadRequest(_) => "bad_request",
        EngineError::Busy => "engine_busy",
        EngineError::Timeout => "engine_timeout",
        EngineError::Spawn(_) => "engine_spawn",
        EngineError::Process(_) | EngineError::Failed { .. } | EngineError::OutcomeUnknown(_) => {
            "engine_failed"
        }
        EngineError::Protocol(_) => "engine_protocol",
    }
}

/// The document of `about`. `engine` is the `engine` object of the engine's `hello` (`version`,
/// `git`, `nvim`); only those three are taken over, whatever else the engine says. `None` when
/// there is no engine or it did not answer: then `engine_problem` says why.
pub(crate) fn about_document(engine: Option<&Value>, engine_problem: Option<&str>) -> Value {
    json!({
        "schema": 1,
        "kind": "hub.about",
        "app": IDENTITY.app,
        "version": crate::version(),
        "engine": engine.filter(|e| e.is_object()).map(|e| json!({
            "version": e["version"],
            "git": e["git"],
            "nvim": e["nvim"],
        })),
        "engine_problem": engine_problem,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_route_is_under_api_and_named_once() {
        let names: Vec<&str> = ROUTES.iter().map(|(n, _)| *n).collect();
        for name in &names {
            assert!(IDENTITY.route(name).starts_with("/api/rules/"), "{name}");
            assert_eq!(names.iter().filter(|n| *n == name).count(), 1, "{name}");
        }
        assert_eq!(
            names,
            [
                "snapshot",
                "rule",
                "sources",
                "about",
                "events",
                "search",
                "rescan",
                "open",
                "autostart",
                "next_id",
                "ops",
                "waivers",
                "rulesets",
                "groupings",
                "views",
                "sources/class"
            ]
        );
    }

    #[test]
    fn every_route_of_the_table_is_a_data_route() {
        let data = &*crate::DATA_ROUTES;
        for (name, _) in ROUTES {
            assert!(
                data.contains(name),
                "{name} is served but not in DATA_ROUTES"
            );
        }
        assert_eq!(data.len(), ROUTES.len());
    }

    #[test]
    fn an_id_is_any_short_text_without_control_characters() {
        for (query, id) in [
            ("id=DEP-01", "DEP-01"),
            ("id=NREL-100", "NREL-100"),
            ("id=a%2Fb", "a/b"),
            ("id=x+y", "x y"),
            ("id=%C3%A4%3A1", "\u{e4}:1"),
            ("id=%2B", "+"),
        ] {
            assert_eq!(rule_id(Some(query)).as_deref(), Ok(id), "{query}");
        }
        let longest = format!("id={}", "A".repeat(MAX_ID_BYTES));
        assert!(rule_id(Some(&longest)).is_ok());
        for bad in [
            None,
            Some(""),
            Some("id"),
            Some("id="),
            Some("x=1"),
            Some("id=a&id=b"),
            Some("id=a&x=b"),
            Some("id=%"),
            Some("id=%4"),
            Some("id=%zz"),
            Some("id=%0A"),
            Some("id=a%00b"),
            Some("id=%7F"),
            Some("id=%C2%85"),
            Some("id=%FF"),
        ] {
            assert!(rule_id(bad).is_err(), "{bad:?}");
        }
        let too_long = format!("id={}", "A".repeat(MAX_ID_BYTES + 1));
        assert!(rule_id(Some(&too_long)).is_err());
        let too_long_escaped = format!("id={}", "%41".repeat(MAX_ID_BYTES + 1));
        assert!(rule_id(Some(&too_long_escaped)).is_err());
    }

    #[test]
    fn about_takes_three_words_of_the_engine_and_nothing_else() {
        let engine = json!({
            "version": "0.1.0",
            "git": "abc1234",
            "nvim": "0.12.2",
            "vault": "C:/Users/someone/vault",
        });
        let doc = about_document(Some(&engine), None);
        assert_eq!(doc["kind"], "hub.about");
        assert_eq!(doc["app"], "rules-desktop");
        assert_eq!(doc["version"], crate::version());
        assert_eq!(doc["engine"]["version"], "0.1.0");
        assert_eq!(doc["engine"]["git"], "abc1234");
        assert_eq!(doc["engine"]["nvim"], "0.12.2");
        assert!(doc["engine_problem"].is_null());
        assert!(!doc.to_string().contains("someone"), "{doc}");
    }

    #[test]
    fn about_without_an_engine_still_names_the_app() {
        let doc = about_document(None, None);
        assert!(doc["engine"].is_null());
        assert_eq!(doc["app"], "rules-desktop");
        assert!(about_document(Some(&json!("text")), None)["engine"].is_null());
        let doc = about_document(None, Some("engine_timeout"));
        assert_eq!(doc["engine_problem"], "engine_timeout");
    }
}
