//! The text of the rules, for the search. The kit's index asks its engine for `bodies` in pages
//! (`{ id, etag, body }` of the open documents). rules.nvim has no such call, so this wrapper answers
//! it from the two calls it has: the `snapshot` names the rules (and their `rev`), `rule` hands over
//! the text of one. Every other call goes to the engine unchanged. `bodies` is never sent to the
//! engine, and the page cannot ask for it: it is not a route.
//!
//! - A page is read from one state of the rule sets: the snapshot and every `rule` document must carry
//!   a `rev`, and every `rule` document the `rev` of the snapshot the ids came from (K9). Else the page
//!   is read again from a new snapshot, at most [`PAGE_TRIES`] times, and then refused; the index keeps
//!   what it had.
//! - The first try of a page takes the snapshot the store holds: a scan of the real rule sets takes about 280 ms in a running
//!   host, and the index reads 100 rules a page, so asking anew cost six scans per change. Only a try after that scans.
//! - A page costs a `rule` question per rule. Without a running host each is a Neovim process of its
//!   own, so with a [`HostProbe`] the texts are only read while the engine's host runs (K10): a host
//!   that is not running is asked one `hello` (which starts a host that can start) and the page is
//!   refused if it still does not run, and the host is looked at again before every rule, so a host
//!   that dies or is left alone while a page is read ends the page instead of a process per rule.

use std::collections::HashSet;
use std::sync::Arc;

use appkit_hub::BoxFuture;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::engine::{CallFuture, Calls, CliEngine, Engine, EngineError, Method, Reply};
use crate::store::SnapshotStore;

/// The most rules read in one page, whatever the index asks for: one question to the engine is
/// asked for each, so a page is as slow as that many questions.
const PAGE_MAX: usize = 100;

/// Reads of a page before it is refused, when the rules keep changing under it.
pub const PAGE_TRIES: usize = 3;

/// Whether the engine's persistent host process answers right now: the texts of the rules are asked
/// one rule at a time, which only a host answers cheaply.
pub trait HostProbe: Send + Sync {
    fn host_running(&self) -> BoxFuture<'_, bool>;
    /// Whether a question can start a host that is not running. An engine without one (one process per question)
    /// never runs it: it is asked no `hello` for that (a whole Neovim process each time).
    fn can_start(&self) -> bool {
        true
    }
}

/// The probe of an engine without a persistent host (`RULES_HUB_ENGINE=process`): the texts of the search are not read.
pub struct NoHost;

impl HostProbe for NoHost {
    fn host_running(&self) -> BoxFuture<'_, bool> {
        Box::pin(async { false })
    }
    fn can_start(&self) -> bool {
        false
    }
}

impl HostProbe for CliEngine {
    fn host_running(&self) -> BoxFuture<'_, bool> {
        Box::pin(CliEngine::host_running(self))
    }
}

/// An engine that also answers `bodies`.
pub struct BodiesEngine {
    inner: Arc<dyn Engine>,
    /// `None`: every question is cheap (no host to wait for), the texts are read.
    host: Option<Arc<dyn HostProbe>>,
    /// The snapshot the hub holds, for the first try of a page; `None`: every try asks the engine.
    store: Option<Arc<SnapshotStore>>,
}

impl BodiesEngine {
    #[must_use]
    pub fn new(inner: Arc<dyn Engine>) -> Self {
        Self {
            inner,
            host: None,
            store: None,
        }
    }

    /// Read a page first from the snapshot `store` holds; `rev` still decides whether the page is of one state.
    #[must_use]
    pub fn with_store(mut self, store: Arc<SnapshotStore>) -> Self {
        self.store = Some(store);
        self
    }

    /// Read the texts only while the host `probe` tells of runs.
    #[must_use]
    pub fn with_host(mut self, probe: Arc<dyn HostProbe>) -> Self {
        self.host = Some(probe);
        self
    }

    async fn host_runs(&self) -> bool {
        match &self.host {
            Some(probe) => probe.host_running().await,
            None => true,
        }
    }
}

#[derive(Deserialize)]
struct Snapshot {
    rev: String,
    rules: Vec<RuleHead>,
}

#[derive(Deserialize)]
struct RuleHead {
    id: String,
    etag: Option<String>,
    #[serde(default)]
    title: String,
}

#[derive(Deserialize)]
struct RuleDoc {
    rev: Option<String>,
    rule: RuleText,
}

#[derive(Deserialize)]
struct RuleText {
    #[serde(default)]
    text: String,
}

fn protocol(why: impl std::fmt::Display) -> EngineError {
    EngineError::Protocol(format!("bodies: {why}"))
}

fn no_host() -> EngineError {
    EngineError::Spawn(
        "bodies: the texts of the rules are read only while the engine's host runs".to_owned(),
    )
}

/// What the index asks for: a page of every rule, or of the named ones.
struct Request {
    ids: Option<Vec<String>>,
    offset: usize,
    limit: usize,
}

impl Request {
    fn read(params: Option<&Value>) -> Self {
        Self {
            offset: params
                .and_then(|p| p["offset"].as_u64())
                .and_then(|n| usize::try_from(n).ok())
                .unwrap_or(0),
            limit: params
                .and_then(|p| p["limit"].as_u64())
                .and_then(|n| usize::try_from(n).ok())
                .map_or(PAGE_MAX, |n| n.clamp(1, PAGE_MAX)),
            ids: params.and_then(|p| p["ids"].as_array()).map(|ids| {
                ids.iter()
                    .filter_map(Value::as_str)
                    .map(str::to_owned)
                    .collect()
            }),
        }
    }
}

impl BodiesEngine {
    async fn bodies(&self, params: Option<&Value>) -> Result<Reply, EngineError> {
        let request = Request::read(params);
        let result = self.read(&request).await;
        result.map(|(_, page)| Reply {
            kind: Calls::Bodies.reply_kind().to_owned(),
            text: page.to_string(),
        })
    }

    async fn read(&self, request: &Request) -> Result<(String, Value), EngineError> {
        if !self.host_runs().await {
            if self.host.as_ref().is_some_and(|probe| !probe.can_start()) {
                return Err(no_host());
            }
            // A host starts with the first question it is asked: one cheap one, so an index built before anything
            // else asked the engine still reads through a host. An engine without a host stays refused.
            let _ = self.inner.call(Calls::Hello, None).await;
            if !self.host_runs().await {
                return Err(no_host());
            }
        }
        for attempt in 0..PAGE_TRIES {
            let held = if attempt == 0 {
                self.store.as_ref().and_then(|store| store.latest())
            } else {
                None
            };
            let was_held = held.is_some();
            let text = match held {
                Some(cached) => cached.text.clone(),
                None => self.inner.call(Calls::Snapshot, None).await?.text.into(),
            };
            // Without its `rev` no page could be told to be of one state.
            let snapshot: Snapshot = serde_json::from_slice(&text)
                .map_err(|e| protocol(format!("a snapshot without its rules or its rev: {e}")))?;
            if let Some(page) = self.read_page(request, snapshot).await? {
                // A page without a rule asked no question that could have seen a newer state: the held snapshot may
                // lag behind, so nothing is the answer only from a fresh one.
                let empty = page.1["items"].as_array().is_some_and(Vec::is_empty);
                if was_held && empty {
                    continue;
                }
                return Ok(page);
            }
        }
        Err(protocol(
            "the rules kept changing while their texts were read",
        ))
    }

    /// One page from one state of the rules; `None` when the state changed while it was read.
    async fn read_page(
        &self,
        request: &Request,
        snapshot: Snapshot,
    ) -> Result<Option<(String, Value)>, EngineError> {
        // The named ids that are rules of the snapshot, each once: no other id reaches the engine.
        let selected: Vec<&RuleHead> = match &request.ids {
            None => snapshot.rules.iter().collect(),
            Some(ids) => {
                let wanted: HashSet<&str> = ids.iter().map(String::as_str).collect();
                snapshot
                    .rules
                    .iter()
                    .filter(|r| wanted.contains(r.id.as_str()))
                    .collect()
            }
        };
        let page: Vec<&&RuleHead> = selected
            .iter()
            .skip(request.offset)
            .take(request.limit)
            .collect();

        let mut items = Vec::with_capacity(page.len());
        for head in &page {
            // Looked at before every rule: a host that died or is left alone would leave each remaining rule to a
            // process of its own.
            if !self.host_runs().await {
                return Err(no_host());
            }
            let asked = json!({ "id": head.id });
            let text = match self.inner.call(Calls::Rule, Some(&asked)).await {
                Ok(reply) => reply.text,
                // The rule went between the snapshot and now: the next change tells the index.
                Err(EngineError::Refused(r)) if r.code == "not_found" => continue,
                Err(other) => return Err(other),
            };
            let doc: RuleDoc = serde_json::from_str(&text).map_err(protocol)?;
            let Some(doc_rev) = doc.rev else {
                return Err(protocol("a rule document without its rev"));
            };
            if doc_rev != snapshot.rev {
                return Ok(None);
            }
            items.push(json!({
                "id": head.id,
                "etag": head.etag,
                // The title is searched too, and the text under it.
                "body": format!("{}\n{}", head.title, doc.rule.text),
            }));
        }
        let end = request.offset.saturating_add(page.len());
        let next_offset = (end < selected.len()).then_some(end);
        let page = json!({
            "schema": 1,
            "kind": "rules.bodies",
            "rev": snapshot.rev,
            "items": items,
            "next_offset": next_offset,
        });
        Ok(Some((snapshot.rev, page)))
    }
}

impl Engine for BodiesEngine {
    fn call<'a>(&'a self, method: Method, params: Option<&'a Value>) -> CallFuture<'a> {
        if method == Calls::Bodies {
            Box::pin(self.bodies(params))
        } else {
            self.inner.call(method, params)
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::Mutex;

    use super::*;
    use crate::engine::EngineRefusal;

    /// Rules by id with a text, the `rev` of the snapshot and of every rule document (which a test may let move
    /// after a number of `rule` answers), and a host that may stop or start; asks are kept.
    pub(crate) struct Fake {
        pub(crate) rules: Mutex<Vec<(&'static str, &'static str)>>,
        pub(crate) rev: Mutex<String>,
        /// After this many `rule` answers the rev moves on once (a change while a page is read).
        move_after: Mutex<Option<usize>>,
        /// Every rule document is of another state than its snapshot.
        always_elsewhere: AtomicBool,
        pub(crate) host: AtomicBool,
        /// `hello` starts the host, as the first question to a `CliEngine` with a host does.
        start_on_hello: AtomicBool,
        /// From this `rule` answer on the host is no longer running.
        host_stops_after: Mutex<Option<usize>>,
        /// The documents carry no `rev`.
        omit_rev: AtomicBool,
        asked: Mutex<Vec<String>>,
        rule_calls: AtomicUsize,
    }

    impl Fake {
        pub(crate) fn new(rules: &[(&'static str, &'static str)]) -> Arc<Self> {
            Arc::new(Self {
                rules: Mutex::new(rules.to_vec()),
                rev: Mutex::new("rev:1".to_owned()),
                move_after: Mutex::new(None),
                always_elsewhere: AtomicBool::new(false),
                host: AtomicBool::new(true),
                start_on_hello: AtomicBool::new(false),
                host_stops_after: Mutex::new(None),
                omit_rev: AtomicBool::new(false),
                asked: Mutex::new(Vec::new()),
                rule_calls: AtomicUsize::new(0),
            })
        }

        pub(crate) fn asked(&self) -> Vec<String> {
            self.asked.lock().unwrap().clone()
        }

        fn count(&self, what: &str) -> usize {
            self.asked().iter().filter(|a| *a == what).count()
        }

        fn stamp(&self, mut doc: Value) -> String {
            if self.omit_rev.load(Ordering::SeqCst) {
                doc.as_object_mut().unwrap().remove("rev");
            }
            doc.to_string()
        }
    }

    impl Engine for Fake {
        fn call<'a>(&'a self, method: Method, params: Option<&'a Value>) -> CallFuture<'a> {
            Box::pin(async move {
                let name = method.as_str();
                self.asked.lock().unwrap().push(name.to_owned());
                let rev = self.rev.lock().unwrap().clone();
                let text = match name {
                    "hello" => {
                        if self.start_on_hello.load(Ordering::SeqCst) {
                            self.host.store(true, Ordering::SeqCst);
                        }
                        "{}".to_owned()
                    }
                    "snapshot" => {
                        let rules: Vec<Value> = self
                            .rules
                            .lock()
                            .unwrap()
                            .iter()
                            .map(|(id, _)| json!({ "id": id, "etag": "e1", "title": format!("title of {id}") }))
                            .collect();
                        self.stamp(json!({ "rev": rev, "rules": rules }))
                    }
                    "rule" => {
                        let n = self.rule_calls.fetch_add(1, Ordering::SeqCst) + 1;
                        if self
                            .host_stops_after
                            .lock()
                            .unwrap()
                            .is_some_and(|k| n >= k)
                        {
                            self.host.store(false, Ordering::SeqCst);
                        }
                        let id = params.and_then(|p| p["id"].as_str()).unwrap();
                        let found = self
                            .rules
                            .lock()
                            .unwrap()
                            .iter()
                            .find(|(i, _)| *i == id)
                            .map(|(_, text)| *text);
                        let Some(text) = found else {
                            return Err(EngineError::Refused(EngineRefusal {
                                code: "not_found".to_owned(),
                                message: format!("no rule with the id {id}"),
                                retryable: false,
                                text: String::new(),
                            }));
                        };
                        let mut moved = self.move_after.lock().unwrap();
                        if moved.is_some_and(|after| n > after) {
                            *moved = None;
                            *self.rev.lock().unwrap() = "rev:2".to_owned();
                        }
                        drop(moved);
                        let rev = if self.always_elsewhere.load(Ordering::SeqCst) {
                            "rev:elsewhere".to_owned()
                        } else {
                            self.rev.lock().unwrap().clone()
                        };
                        self.stamp(json!({ "rev": rev, "rule": { "id": id, "text": text } }))
                    }
                    other => panic!("asked {other}"),
                };
                Ok(Reply {
                    kind: method.reply_kind().to_owned(),
                    text,
                })
            })
        }
    }

    impl HostProbe for Fake {
        fn host_running(&self) -> BoxFuture<'_, bool> {
            Box::pin(async { self.host.load(Ordering::SeqCst) })
        }
    }

    /// A `BodiesEngine` over `fake` that looks at its host.
    pub(crate) fn hosted(fake: &Arc<Fake>) -> BodiesEngine {
        BodiesEngine::new(Arc::clone(fake) as Arc<dyn Engine>)
            .with_host(Arc::clone(fake) as Arc<dyn HostProbe>)
    }

    fn three() -> Arc<Fake> {
        Fake::new(&[
            ("A-01", "text of A-01"),
            ("A-02", "text of A-02"),
            ("B-01", "text of B-01"),
        ])
    }

    async fn page(e: &BodiesEngine, params: Value) -> Result<Value, EngineError> {
        let reply = e.call(Calls::Bodies, Some(&params)).await?;
        assert_eq!(reply.kind, "rules.bodies");
        Ok(serde_json::from_str(&reply.text).unwrap())
    }

    #[tokio::test]
    async fn a_page_holds_the_title_and_the_text_of_each_rule() {
        let e = hosted(&three());
        let doc = page(&e, json!({ "offset": 0, "limit": 1000 }))
            .await
            .unwrap();
        assert_eq!(doc["rev"], "rev:1");
        assert_eq!(doc["items"].as_array().unwrap().len(), 3);
        assert_eq!(doc["items"][0]["id"], "A-01");
        assert_eq!(doc["items"][0]["etag"], "e1");
        assert_eq!(doc["items"][0]["body"], "title of A-01\ntext of A-01");
        assert!(doc["next_offset"].is_null());
    }

    #[tokio::test]
    async fn pages_follow_each_other_by_offset() {
        let e = hosted(&three());
        let first = page(&e, json!({ "offset": 0, "limit": 2 })).await.unwrap();
        assert_eq!(first["items"].as_array().unwrap().len(), 2);
        assert_eq!(first["next_offset"], 2);
        let second = page(&e, json!({ "offset": 2, "limit": 2 })).await.unwrap();
        assert_eq!(second["items"][0]["id"], "B-01");
        assert!(second["next_offset"].is_null());
    }

    #[tokio::test]
    async fn named_ids_are_the_only_ones_read() {
        let fake = three();
        let e = hosted(&fake);
        let doc = page(
            &e,
            json!({ "ids": ["B-01", "nope"], "offset": 0, "limit": 10 }),
        )
        .await
        .unwrap();
        let ids: Vec<&str> = doc["items"]
            .as_array()
            .unwrap()
            .iter()
            .map(|i| i["id"].as_str().unwrap())
            .collect();
        assert_eq!(ids, ["B-01"]);
        assert_eq!(
            fake.count("rule"),
            1,
            "a rule that was not asked for was read"
        );
    }

    #[tokio::test]
    async fn a_limit_of_zero_is_a_page_of_one() {
        let e = hosted(&three());
        let doc = page(&e, json!({ "limit": 0 })).await.unwrap();
        assert_eq!(
            doc["items"].as_array().unwrap().len(),
            1,
            "a limit of 0 is 1"
        );
    }

    #[tokio::test]
    async fn every_other_call_goes_to_the_engine_unchanged() {
        let fake = three();
        let e = hosted(&fake);
        let reply = e.call(Calls::Snapshot, None).await.unwrap();
        assert!(reply.text.contains("rev:1"));
        assert_eq!(fake.asked(), ["snapshot"]);
    }

    fn store_of(fake: &Arc<Fake>) -> Arc<SnapshotStore> {
        SnapshotStore::new(
            Some(fake.clone() as Arc<dyn Engine>),
            crate::store::SHAPE,
            crate::store::StoreOptions::default(),
        )
    }

    #[tokio::test]
    async fn a_page_takes_the_ids_of_the_snapshot_the_store_holds() {
        let fake = three();
        let store = store_of(&fake);
        store.current().await.unwrap();
        let scans = |fake: &Fake| fake.asked().iter().filter(|m| *m == "snapshot").count();
        assert_eq!(scans(&fake), 1);
        let e = hosted(&fake).with_store(store);
        let doc = page(&e, json!({ "offset": 0, "limit": 10 })).await.unwrap();
        assert_eq!(doc["items"].as_array().unwrap().len(), 3);
        assert_eq!(scans(&fake), 1, "{:?}", fake.asked());

        // The held snapshot is older than the rules: the page is read again from a fresh one.
        *fake.move_after.lock().unwrap() = Some(1);
        let doc = page(&e, json!({ "offset": 0, "limit": 10 })).await.unwrap();
        assert_eq!(doc["rev"], "rev:2");
        assert_eq!(scans(&fake), 2, "{:?}", fake.asked());
    }

    #[tokio::test]
    async fn a_page_with_no_rule_is_not_taken_from_the_held_snapshot_alone() {
        let fake = three();
        let store = store_of(&fake);
        store.current().await.unwrap();
        let e = hosted(&fake).with_store(store);
        let scans = |fake: &Fake| fake.asked().iter().filter(|m| *m == "snapshot").count();
        assert_eq!(scans(&fake), 1);
        // Past the end: no question could have seen that the held snapshot lags behind.
        let doc = page(&e, json!({ "offset": 99, "limit": 10 }))
            .await
            .unwrap();
        assert!(doc["items"].as_array().unwrap().is_empty());
        assert_eq!(scans(&fake), 2, "{:?}", fake.asked());
    }

    #[tokio::test]
    async fn a_page_read_across_a_change_is_read_again() {
        let fake = three();
        // The second rule document is of the next state.
        *fake.move_after.lock().unwrap() = Some(1);
        let e = hosted(&fake);
        let doc = page(&e, json!({ "offset": 0, "limit": 10 })).await.unwrap();
        assert_eq!(doc["rev"], "rev:2");
        assert_eq!(doc["items"].as_array().unwrap().len(), 3);
        assert_eq!(fake.count("snapshot"), 2, "read again from a new snapshot");
    }

    #[tokio::test]
    async fn a_page_that_never_holds_still_is_refused() {
        let fake = three();
        fake.always_elsewhere.store(true, Ordering::SeqCst);
        let e = hosted(&fake);
        let err = page(&e, json!({ "offset": 0, "limit": 10 }))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("kept changing"), "{err}");
        assert_eq!(fake.count("snapshot"), PAGE_TRIES);
    }

    #[tokio::test]
    async fn documents_without_their_rev_are_refused() {
        let fake = three();
        fake.omit_rev.store(true, Ordering::SeqCst);
        let e = hosted(&fake);
        let err = page(&e, json!({ "offset": 0, "limit": 10 }))
            .await
            .unwrap_err();
        assert!(matches!(err, EngineError::Protocol(_)), "{err}");

        // A snapshot with its rev and rule documents without theirs: None is no rev.
        struct NoRuleRev(Arc<Fake>);
        impl Engine for NoRuleRev {
            fn call<'a>(&'a self, method: Method, params: Option<&'a Value>) -> CallFuture<'a> {
                Box::pin(async move {
                    let mut reply = self.0.call(method, params).await?;
                    if method == Calls::Rule {
                        let mut doc: Value = serde_json::from_str(&reply.text).unwrap();
                        doc.as_object_mut().unwrap().remove("rev");
                        reply.text = doc.to_string();
                    }
                    Ok(reply)
                })
            }
        }
        let e = BodiesEngine::new(Arc::new(NoRuleRev(three())) as Arc<dyn Engine>);
        let err = page(&e, json!({ "offset": 0, "limit": 10 }))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("without its rev"), "{err}");
    }

    #[tokio::test]
    async fn a_host_that_stops_while_a_page_is_read_ends_the_page() {
        let fake = three();
        *fake.host_stops_after.lock().unwrap() = Some(1);
        let e = hosted(&fake);
        let err = page(&e, json!({ "offset": 0, "limit": 10 }))
            .await
            .unwrap_err();
        assert!(matches!(err, EngineError::Spawn(_)), "{err}");
        assert_eq!(
            fake.count("rule"),
            1,
            "no rule after the host stopped: {:?}",
            fake.asked()
        );
    }

    #[tokio::test]
    async fn a_host_that_is_not_started_yet_is_started_by_a_hello() {
        let fake = three();
        fake.host.store(false, Ordering::SeqCst);
        fake.start_on_hello.store(true, Ordering::SeqCst);
        let e = hosted(&fake);
        let doc = page(&e, json!({ "offset": 0, "limit": 10 })).await.unwrap();
        assert_eq!(doc["items"].as_array().unwrap().len(), 3);
        assert_eq!(fake.count("hello"), 1, "exactly one hello");
    }

    #[tokio::test]
    async fn an_engine_that_has_no_host_at_all_is_asked_no_hello() {
        let fake = three();
        let e = BodiesEngine::new(Arc::clone(&fake) as Arc<dyn Engine>).with_host(Arc::new(NoHost));
        let err = page(&e, json!({ "offset": 0, "limit": 10 }))
            .await
            .unwrap_err();
        assert!(matches!(err, EngineError::Spawn(_)), "{err}");
        assert!(fake.asked().is_empty(), "{:?}", fake.asked());
    }

    #[tokio::test]
    async fn an_engine_without_a_host_reads_no_text() {
        let fake = three();
        fake.host.store(false, Ordering::SeqCst);
        let e = hosted(&fake);
        let err = page(&e, json!({ "offset": 0, "limit": 10 }))
            .await
            .unwrap_err();
        assert!(matches!(err, EngineError::Spawn(_)), "{err}");
        assert_eq!(
            fake.count("hello"),
            1,
            "one hello, not one per page attempt"
        );
        assert_eq!(fake.count("rule"), 0, "no rule without a host");
    }
}
