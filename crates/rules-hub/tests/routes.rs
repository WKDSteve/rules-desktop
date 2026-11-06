//! The read routes of the hub against an engine in the process, without Neovim: a fake that answers
//! with the golden documents of rules.nvim (`tests/fixtures/golden`, a copy of
//! `schemas/rules-export/1/golden` at the pinned commit; CI compares the two) and counts what it
//! was asked.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use appkit_core::authn::{ManualClock, Secret, CODE_BYTES, TOKEN_BYTES};
use appkit_hub::engine::{CallFuture, Engine, EngineError, EngineRefusal, Method, Reply};
use appkit_hub::{HubStatus, Limits, RunningServer};
use rules_hub::sources::{Class, SourcePolicy};
use rules_hub::Watch;
use serde_json::{json, Value};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

mod common;

use common::scratch::Scratch;
use common::{get, get_with, post, session};

const HELLO: &str = include_str!("fixtures/golden/hello.json");
const SNAPSHOT: &str = include_str!("fixtures/golden/snapshot.json");
const RULE: &str = include_str!("fixtures/golden/rule.json");
const ERROR: &str = include_str!("fixtures/golden/error.json");
const NEXT_ID: &str = include_str!("fixtures/golden/next_id.json");
const RESULT: &str = include_str!("fixtures/golden/result.json");
const WAIVERS: &str = include_str!("fixtures/golden/waivers.json");
const RULESETS: &str = include_str!("fixtures/golden/rulesets.json");
/// What the hub answers `sources` with for the golden snapshot; the page's tests answer with it.
const HUB_SOURCES: &str = include_str!("fixtures/hub/sources.json");

/// The digest of the golden snapshot: its ETag.
const SNAPSHOT_DIGEST: &str = "sha256:9609c01610189534";

/// An engine in the process. It answers `hello` and `snapshot` with the golden documents, `rule`
/// with the golden rule for its id (`DEP-01`) and a `not_found` refusal for any other, and keeps
/// every question.
struct FakeEngine {
    snapshot: Mutex<String>,
    hello: Mutex<Result<String, ()>>,
    rule: Mutex<String>,
    /// What `ops` answers: the golden dry run, unless a test says what a write answers.
    ops: Mutex<String>,
    /// While set, `snapshot` waits for a permit of it.
    snapshot_gate: Mutex<Option<Arc<tokio::sync::Semaphore>>>,
    asked: Mutex<Vec<(&'static str, Option<Value>)>>,
}

impl FakeEngine {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            snapshot: Mutex::new(SNAPSHOT.trim_end().to_owned()),
            hello: Mutex::new(Ok(HELLO.trim_end().to_owned())),
            rule: Mutex::new(RULE.trim_end().to_owned()),
            ops: Mutex::new(RESULT.trim_end().to_owned()),
            snapshot_gate: Mutex::new(None),
            asked: Mutex::new(Vec::new()),
        })
    }

    fn count(&self, method: &str) -> usize {
        self.asked
            .lock()
            .unwrap()
            .iter()
            .filter(|(m, _)| *m == method)
            .count()
    }

    fn params_of(&self, method: &str) -> Vec<Option<Value>> {
        self.asked
            .lock()
            .unwrap()
            .iter()
            .filter(|(m, _)| *m == method)
            .map(|(_, p)| p.clone())
            .collect()
    }

    fn not_found(id: &str) -> EngineError {
        let doc: Value = serde_json::from_str(ERROR).unwrap();
        EngineError::Refused(EngineRefusal {
            code: doc["code"].as_str().unwrap().to_owned(),
            message: format!("no rule with the id {id}"),
            retryable: false,
            text: ERROR.trim_end().to_owned(),
        })
    }
}

impl Engine for FakeEngine {
    fn call<'a>(&'a self, method: Method, params: Option<&'a Value>) -> CallFuture<'a> {
        Box::pin(async move {
            self.asked
                .lock()
                .unwrap()
                .push((method.as_str(), params.cloned()));
            let text = match method.as_str() {
                "hello" => self
                    .hello
                    .lock()
                    .unwrap()
                    .clone()
                    .map_err(|()| EngineError::Timeout)?,
                "snapshot" => {
                    let gate = self.snapshot_gate.lock().unwrap().clone();
                    if let Some(gate) = gate {
                        drop(gate.acquire().await.unwrap());
                    }
                    self.snapshot.lock().unwrap().clone()
                }
                "rule" => {
                    let id = params.and_then(|p| p["id"].as_str()).unwrap_or_default();
                    if id != "DEP-01" {
                        return Err(Self::not_found(id));
                    }
                    self.rule.lock().unwrap().clone()
                }
                "next_id" => NEXT_ID.trim_end().to_owned(),
                "waivers" => WAIVERS.trim_end().to_owned(),
                "rulesets" => RULESETS.trim_end().to_owned(),
                "ops" => self.ops.lock().unwrap().clone(),
                other => panic!("the fake engine was asked {other}"),
            };
            Ok(Reply {
                kind: method.reply_kind().to_owned(),
                text,
            })
        })
    }
}

struct Hub {
    server: RunningServer,
    token: String,
    /// The control secret: a test pairs further sessions with it.
    control: Secret,
}

fn ready() -> HubStatus {
    HubStatus {
        engine_ready: true,
        problem: None,
        capabilities: vec![
            "hello".into(),
            "snapshot".into(),
            "rule".into(),
            "next_id".into(),
            "ops".into(),
            "waivers".into(),
        ],
    }
}

async fn start_with(engine: Option<Arc<FakeEngine>>, max_age: Option<Duration>) -> Hub {
    start_classified(engine, max_age, Arc::default(), None).await
}

async fn start_classified(
    engine: Option<Arc<FakeEngine>>,
    max_age: Option<Duration>,
    sources: Arc<SourcePolicy>,
    watch: Option<Watch>,
) -> Hub {
    start_full(engine, max_age, sources, watch, None).await
}

async fn start_full(
    engine: Option<Arc<FakeEngine>>,
    max_age: Option<Duration>,
    sources: Arc<SourcePolicy>,
    watch: Option<Watch>,
    editor: Option<rules_hub::Editor>,
) -> Hub {
    let limits = Limits {
        snapshot_max_age: max_age,
        ..Limits::default()
    };
    start_limited(engine, limits, sources, watch, editor).await
}

async fn start_limited(
    engine: Option<Arc<FakeEngine>>,
    limits: Limits,
    sources: Arc<SourcePolicy>,
    watch: Option<Watch>,
    editor: Option<rules_hub::Editor>,
) -> Hub {
    start_all(engine, limits, sources, watch, editor, None, None).await
}

async fn start_all(
    engine: Option<Arc<FakeEngine>>,
    limits: Limits,
    sources: Arc<SourcePolicy>,
    watch: Option<Watch>,
    editor: Option<rules_hub::Editor>,
    project: Option<std::path::PathBuf>,
    config_dir: Option<std::path::PathBuf>,
) -> Hub {
    start_login(
        engine, limits, sources, watch, editor, project, config_dir, None,
    )
    .await
}

/// A login entry that holds its value (no test touches the registry of the machine).
struct FakeLogin {
    on: Mutex<Option<bool>>,
    refuse: bool,
}

impl rules_hub::loginentry::LoginEntry for FakeLogin {
    fn is_on(&self) -> Option<bool> {
        *self.on.lock().unwrap()
    }

    fn set(&self, on: bool) -> Result<(), String> {
        if self.refuse {
            return Err("the system said no".to_owned());
        }
        *self.on.lock().unwrap() = Some(on);
        Ok(())
    }
}

#[allow(clippy::too_many_arguments)]
async fn start_login(
    engine: Option<Arc<FakeEngine>>,
    limits: Limits,
    sources: Arc<SourcePolicy>,
    watch: Option<Watch>,
    editor: Option<rules_hub::Editor>,
    project: Option<std::path::PathBuf>,
    config_dir: Option<std::path::PathBuf>,
    login: Option<Arc<dyn rules_hub::loginentry::LoginEntry>>,
) -> Hub {
    let control = Secret::generate(TOKEN_BYTES).unwrap();
    let server = rules_hub::start(rules_hub::Start {
        addr: "127.0.0.1:0".parse().unwrap(),
        limits,
        clock: Arc::new(ManualClock::new()),
        pairing_ttl: Duration::from_secs(60),
        control_secret: control.clone(),
        instance: Secret::generate(CODE_BYTES).unwrap(),
        status: if engine.is_some() {
            ready()
        } else {
            HubStatus {
                engine_ready: false,
                problem: Some("engine_not_found"),
                capabilities: Vec::new(),
            }
        },
        engine: engine.map(|e| e as Arc<dyn Engine>),
        host: None,
        sources,
        watch,
        editor,
        login: login.clone(),
        ui: rules_hub::ui::UiFiles::none(),
        project,
        config_dir,
    })
    .await
    .expect("the hub starts");
    let token = session(&server, &control).await;
    Hub {
        server,
        token,
        control,
    }
}

async fn start(engine: &Arc<FakeEngine>) -> Hub {
    // The snapshot is held until something says it changed: each fetch is one the test caused.
    start_with(Some(Arc::clone(engine)), None).await
}

fn route(name: &str) -> String {
    rules_hub::IDENTITY.route(name)
}

impl Hub {
    async fn get(&self, path: &str) -> appkit_testkit::wire::Reply {
        get(&self.server, path, Some(&self.token)).await
    }

    async fn post(&self, path: &str, body: &str) -> appkit_testkit::wire::Reply {
        post(&self.server, path, Some(&self.token), body).await
    }
}

#[tokio::test]
async fn the_login_entry_is_read_set_and_taken_away_and_only_with_a_session() {
    let start_it = |login: Arc<dyn rules_hub::loginentry::LoginEntry>| {
        start_login(
            None,
            Limits::default(),
            Arc::default(),
            None,
            None,
            None,
            None,
            Some(login),
        )
    };
    let login = Arc::new(FakeLogin {
        on: Mutex::new(Some(false)),
        refuse: false,
    });
    let hub = start_it(login.clone()).await;
    let at = route("autostart");
    assert!(hub.get(&at).await.body.contains("false"));
    let on = hub.post(&at, r#"{"on":true}"#).await;
    assert_eq!(on.status, 200, "{on:?}");
    assert_eq!(*login.on.lock().unwrap(), Some(true));
    assert!(hub.get(&at).await.body.contains("true"));
    for bad in [
        "{}",
        r#"{"on":"yes"}"#,
        r#"{"on":true,"exe":"x"}"#,
        "nonsense",
    ] {
        assert_eq!(hub.post(&at, bad).await.status, 400, "{bad}");
    }
    assert_eq!(get(&hub.server, &at, None).await.status, 401);
    assert_eq!(
        *login.on.lock().unwrap(),
        Some(true),
        "a refused request changed nothing"
    );

    let refusing = Arc::new(FakeLogin {
        on: Mutex::new(Some(false)),
        refuse: true,
    });
    let told = start_it(refusing).await.post(&at, r#"{"on":true}"#).await;
    assert_eq!(told.status, 502, "{told:?}");
    assert!(told.body.contains("autostart_failed"));

    let none = start_with(None, None).await;
    assert!(none.get(&at).await.body.contains("null"));
    assert_eq!(none.post(&at, r#"{"on":true}"#).await.status, 404);
}

#[tokio::test]
async fn every_route_of_the_table_is_served_to_a_session_and_to_nobody_else() {
    let engine = FakeEngine::new();
    let hub = start(&engine).await;
    let mut seen = HashMap::new();
    for name in rules_hub::DATA_ROUTES.iter() {
        let without = get(&hub.server, &route(name), None).await;
        assert_eq!(without.status, 401, "{name}: {without:?}");
        // The stream does not end and the rescan is a POST: both are tried on their own below.
        if matches!(
            *name,
            "events" | "rescan" | "open" | "ops" | "sources/class" | "autostart"
        ) {
            continue;
        }
        let with = hub.get(&route(name)).await;
        // Registered: a route that is not would be a 404 of the router with an empty body.
        // `waivers` without a project of the device is a 409 that says so.
        assert!([200, 400, 409].contains(&with.status), "{name}: {with:?}");
        seen.insert(*name, with.status);
    }
    assert_eq!(seen.get("snapshot"), Some(&200));
    // `rule` without an id is a bad query, told by the route.
    assert_eq!(seen.get("rule"), Some(&400));
    assert_eq!(seen.get("sources"), Some(&200));
    assert_eq!(seen.get("about"), Some(&200));
    assert_eq!(seen.get("next_id"), Some(&200));
    assert_eq!(seen.get("waivers"), Some(&409));
    assert_eq!(seen.get("rulesets"), Some(&200));
    // `search` without a query is a bad query, told by the route.
    assert_eq!(seen.get("search"), Some(&400));
}

#[tokio::test]
async fn the_snapshot_is_the_engine_s_document_with_its_digest_as_etag() {
    let engine = FakeEngine::new();
    let hub = start(&engine).await;
    let r = hub.get(&route("snapshot")).await;
    assert_eq!(r.status, 200, "{r:?}");
    assert_eq!(r.body, SNAPSHOT.trim_end(), "passed on byte for byte");
    let quoted = format!("\"{SNAPSHOT_DIGEST}\"");
    assert_eq!(r.header("etag"), Some(quoted.as_str()));
    assert_eq!(r.header("content-type"), Some("application/json"));

    // The tag the client holds: nothing new, no body.
    for held in [
        quoted.clone(),
        format!("W/{quoted}"),
        "\"x\", ".to_owned() + &quoted,
    ] {
        let again = get_with(
            &hub.server,
            &route("snapshot"),
            Some(&hub.token),
            &[("If-None-Match", held.as_str())],
        )
        .await;
        assert_eq!(again.status, 304, "{held}: {again:?}");
        assert!(again.body.is_empty(), "{again:?}");
        assert_eq!(again.header("etag"), Some(quoted.as_str()));
    }
    let other = get_with(
        &hub.server,
        &route("snapshot"),
        Some(&hub.token),
        &[("If-None-Match", "\"sha256:0000000000000000\"")],
    )
    .await;
    assert_eq!(other.status, 200);
    assert_eq!(other.body, SNAPSHOT.trim_end());
    // The snapshot was read once and held for every answer.
    assert_eq!(engine.count("snapshot"), 1);

    let with_query = hub.get(&format!("{}?x=1", route("snapshot"))).await;
    assert_eq!(with_query.status, 400, "{with_query:?}");
}

#[tokio::test]
async fn a_rule_of_the_snapshot_is_the_engine_s_rule_document() {
    let engine = FakeEngine::new();
    let hub = start(&engine).await;
    let r = hub.get(&format!("{}?id=DEP-01", route("rule"))).await;
    assert_eq!(r.status, 200, "{r:?}");
    assert_eq!(r.body, RULE.trim_end(), "passed on byte for byte");
    assert_eq!(r.json()["rule"]["text"], "Text of one.");
    assert_eq!(engine.params_of("rule"), [Some(json!({ "id": "DEP-01" }))]);
}

#[tokio::test]
async fn an_id_that_is_not_in_the_snapshot_never_reaches_the_engine() {
    let engine = FakeEngine::new();
    let hub = start(&engine).await;
    for id in [
        "NOPE-1",
        "dep-01",
        "DEP-01%20",
        "..%2F..%2Fetc%2Fpasswd",
        "%22%7D%2C%22x%22%3A%7B",
        "DEP-01%2Bx",
    ] {
        let r = hub.get(&format!("{}?id={id}", route("rule"))).await;
        assert_eq!(r.status, 404, "{id}: {r:?}");
        assert_eq!(r.json()["error"]["code"], "not_found", "{id}");
    }
    assert_eq!(
        engine.count("rule"),
        0,
        "no foreign string reached the engine"
    );
    assert_eq!(engine.count("snapshot"), 1);
}

#[tokio::test]
async fn a_query_that_is_no_id_is_refused_before_anything_is_asked() {
    let engine = FakeEngine::new();
    let hub = start(&engine).await;
    let long = "A".repeat(rules_hub::MAX_ID_BYTES + 1);
    for query in [
        String::new(),
        "?id=".to_owned(),
        "?x=DEP-01".to_owned(),
        "?id=DEP-01&id=DEP-09".to_owned(),
        "?id=DEP-01&x=1".to_owned(),
        "?id=%0ADEP-01".to_owned(),
        "?id=%FF".to_owned(),
        "?id=%G1".to_owned(),
        format!("?id={long}"),
    ] {
        let r = hub.get(&format!("{}{query}", route("rule"))).await;
        assert_eq!(r.status, 400, "{query}: {r:?}");
        assert_eq!(r.json()["error"]["code"], "bad_query", "{query}");
    }
    assert_eq!(engine.count("rule"), 0);
    assert_eq!(
        engine.count("snapshot"),
        0,
        "refused before the snapshot is read"
    );
}

#[tokio::test]
async fn a_rule_the_engine_no_longer_has_is_its_not_found() {
    // DEP-09 is in the snapshot; the fake engine has no document for it, as when the rule was
    // removed after the snapshot was read.
    let engine = FakeEngine::new();
    let hub = start(&engine).await;
    let r = hub.get(&format!("{}?id=DEP-09", route("rule"))).await;
    assert_eq!(r.status, 404, "{r:?}");
    assert_eq!(r.json()["error"]["code"], "not_found");
    assert_eq!(r.json()["error"]["message"], "no rule with the id DEP-09");
    assert_eq!(engine.count("rule"), 1);
}

#[tokio::test]
async fn the_ids_follow_the_snapshot_the_hub_holds() {
    let engine = FakeEngine::new();
    // A snapshot without DEP-01 first.
    let mut doc: Value = serde_json::from_str(SNAPSHOT).unwrap();
    doc["rules"]
        .as_array_mut()
        .unwrap()
        .retain(|r| r["id"] != "DEP-01");
    doc["digest"] = json!("sha256:1111111111111111");
    *engine.snapshot.lock().unwrap() = doc.to_string();
    // Every request reads the snapshot again.
    let hub = start_with(Some(Arc::clone(&engine)), Some(Duration::ZERO)).await;
    let path = format!("{}?id=DEP-01", route("rule"));
    assert_eq!(hub.get(&path).await.status, 404);
    assert_eq!(engine.count("rule"), 0);
    // The rule arrives in the next snapshot: now it is asked for.
    *engine.snapshot.lock().unwrap() = SNAPSHOT.trim_end().to_owned();
    let r = hub.get(&path).await;
    assert_eq!(r.status, 200, "{r:?}");
    assert_eq!(engine.count("rule"), 1);
}

#[tokio::test]
async fn a_snapshot_without_a_list_of_rules_asks_no_rule() {
    let engine = FakeEngine::new();
    *engine.snapshot.lock().unwrap() =
        r#"{"digest":"sha256:2222222222222222","kind":"rules.snapshot","schema":1}"#.to_owned();
    let hub = start(&engine).await;
    let r = hub.get(&format!("{}?id=DEP-01", route("rule"))).await;
    assert_eq!(r.status, 502, "{r:?}");
    assert_eq!(r.json()["error"]["code"], "engine_failed");
    assert_eq!(engine.count("rule"), 0);
}

#[tokio::test]
async fn without_an_engine_the_data_routes_say_so_and_about_still_answers() {
    let hub = start_with(None, None).await;
    for path in [
        route("snapshot"),
        format!("{}?id=DEP-01", route("rule")),
        route("sources"),
    ] {
        let r = hub.get(&path).await;
        assert_eq!(r.status, 503, "{path}: {r:?}");
        assert_eq!(r.json()["error"]["code"], "engine_unavailable");
        assert_eq!(r.json()["error"]["problem"], "engine_not_found");
    }
    let about = hub.get(&route("about")).await;
    assert_eq!(about.status, 200, "{about:?}");
    assert_eq!(about.json()["app"], "rules-desktop");
    assert!(about.json()["engine"].is_null());
    assert!(about.json()["engine_problem"].is_null());
}

#[tokio::test]
async fn about_names_the_engine_from_its_hello() {
    let engine = FakeEngine::new();
    let hub = start(&engine).await;
    let r = hub.get(&route("about")).await;
    assert_eq!(r.status, 200, "{r:?}");
    let doc = r.json();
    assert_eq!(doc["kind"], "hub.about");
    assert_eq!(doc["app"], "rules-desktop");
    assert_eq!(doc["version"], rules_hub::version());
    assert_eq!(
        doc["engine"],
        json!({ "version": "0.1.0", "git": "golden", "nvim": "golden" })
    );
    assert!(doc["engine_problem"].is_null());
    assert_eq!(engine.count("hello"), 1);

    // An engine that does not answer is told apart from no engine.
    *engine.hello.lock().unwrap() = Err(());
    let r = hub.get(&route("about")).await;
    assert_eq!(r.status, 200, "{r:?}");
    assert!(r.json()["engine"].is_null());
    assert_eq!(r.json()["engine_problem"], "engine_timeout");

    // A hello without an engine object is a broken engine, not a missing one.
    for hello in [
        r#"{"schema":1,"kind":"rules.hello"}"#,
        r#"{"engine":"text"}"#,
    ] {
        *engine.hello.lock().unwrap() = Ok(hello.to_owned());
        let r = hub.get(&route("about")).await;
        assert_eq!(r.status, 200, "{r:?}");
        assert!(r.json()["engine"].is_null(), "{hello}");
        assert_eq!(r.json()["engine_problem"], "engine_protocol", "{hello}");
    }
}

#[tokio::test]
async fn the_probe_accepts_the_golden_hello() {
    let engine = FakeEngine::new();
    let info = rules_hub::engine::probe(engine.as_ref()).await.unwrap();
    assert!(info.schemas.contains(&rules_hub::engine::SCHEMA));
    for needed in rules_hub::engine::REQUIRED_CAPABILITIES {
        assert!(info.capabilities.iter().any(|c| c == needed), "{needed}");
    }
}

fn policy(keys: &[(&str, Class)], default: Option<Class>) -> Arc<SourcePolicy> {
    Arc::new(
        SourcePolicy::new(keys.iter().map(|(k, c)| ((*k).to_owned(), *c)))
            .unwrap()
            .with_default(default),
    )
}

/// The golden snapshot with other files and one rule in each.
fn snapshot_with_files(paths: &[&str]) -> String {
    let mut doc: Value = serde_json::from_str(SNAPSHOT).unwrap();
    doc["sources"] = paths
        .iter()
        .map(|p| json!({ "etag": "sha256:0000000000000000", "path": p, "rules": 1 }))
        .collect();
    doc["digest"] = json!("sha256:3333333333333333");
    doc.to_string()
}

#[tokio::test]
async fn without_sources_json_every_file_is_unclassified() {
    let engine = FakeEngine::new();
    let hub = start(&engine).await;
    let r = hub.get(&route("sources")).await;
    assert_eq!(r.status, 200, "{r:?}");
    let doc = r.json();
    assert_eq!(doc["kind"], "hub.sources");
    assert_eq!(
        doc["files"],
        json!([
            { "path": "Checklists/DEP.md", "data_class": "unclassified", "rules": 2, "key": null },
            { "path": "Checklists/LUA.md", "data_class": "unclassified", "rules": 1, "key": null },
            { "path": "Checklists/_rules/ERR/ERR-100.md", "data_class": "unclassified",
              "rules": 1, "key": null },
        ])
    );
    assert_eq!(doc["keys"], json!([]));
    assert_eq!(doc["unclassified"].as_array().unwrap().len(), 3);
    assert_eq!(doc["private"], json!([]));
    // Computed from the held snapshot: one read for both answers, nothing else asked.
    assert_eq!(hub.get(&route("sources")).await.status, 200);
    assert_eq!(engine.count("snapshot"), 1);
    assert_eq!(engine.asked.lock().unwrap().len(), 1);

    let with_query = hub.get(&format!("{}?x=1", route("sources"))).await;
    assert_eq!(with_query.status, 400, "{with_query:?}");
}

#[tokio::test]
async fn sources_gives_each_file_its_class_and_the_strictest_of_two_keys() {
    let engine = FakeEngine::new();
    *engine.snapshot.lock().unwrap() = snapshot_with_files(&[
        "wkdbook-Lua/Checklists/DEP.md",
        "wkdbook-Lua/README.md",
        "Checklists#2/DEP.md",
        "lua/DEP.md",
        "native/REL.md",
    ]);
    let classes = policy(
        &[
            ("wkdbook-Lua", Class::Public),
            ("wkdbook-Lua/Checklists", Class::Private),
            ("Checklists", Class::Public),
            ("lua", Class::Internal),
        ],
        Some(Class::Internal),
    );
    let hub = start_classified(Some(Arc::clone(&engine)), None, classes, None).await;
    let r = hub.get(&route("sources")).await;
    assert_eq!(r.status, 200, "{r:?}");
    let doc = r.json();
    let shown: Vec<(&str, &str, Option<&str>)> = doc["files"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| {
            (
                f["path"].as_str().unwrap(),
                f["data_class"].as_str().unwrap(),
                f["key"].as_str(),
            )
        })
        .collect();
    assert_eq!(
        shown,
        [
            // Two keys count, the strictest wins.
            (
                "wkdbook-Lua/Checklists/DEP.md",
                "private",
                Some("wkdbook-Lua/Checklists")
            ),
            ("wkdbook-Lua/README.md", "public", Some("wkdbook-Lua")),
            // A numbered name, not even the default.
            ("Checklists#2/DEP.md", "unclassified", None),
            ("lua/DEP.md", "internal", Some("lua")),
            ("native/REL.md", "internal", None),
        ]
    );
    assert_eq!(doc["unclassified"], json!(["Checklists#2/DEP.md"]));
    assert_eq!(doc["private"], json!(["wkdbook-Lua/Checklists/DEP.md"]));
    assert_eq!(
        doc["keys"],
        json!([
            { "key": "Checklists", "class": "public", "files": 1 },
            { "key": "lua", "class": "internal", "files": 1 },
            { "key": "wkdbook-Lua", "class": "public", "files": 2 },
            { "key": "wkdbook-Lua/Checklists", "class": "private", "files": 1 },
        ])
    );
}

/// The page's tests answer `sources` with `tests/fixtures/hub/sources.json` (ui/src/lib/fixtures.ts
/// imports it): it must be what the hub writes for the golden snapshot with the policy named there.
#[tokio::test]
async fn the_sources_document_of_the_page_tests_is_what_the_hub_writes() {
    let engine = FakeEngine::new();
    let classes = policy(
        &[
            ("Checklists", Class::Internal),
            ("Checklists/_rules", Class::Private),
        ],
        None,
    );
    let hub = start_classified(Some(Arc::clone(&engine)), None, classes, None).await;
    let r = hub.get(&route("sources")).await;
    assert_eq!(r.status, 200, "{r:?}");
    let written: Value = serde_json::from_str(HUB_SOURCES).unwrap();
    assert_eq!(r.json(), written, "the hub now writes {:#}", r.json());
}

#[tokio::test]
async fn a_snapshot_without_its_files_is_no_empty_list() {
    let engine = FakeEngine::new();
    let mut doc: Value = serde_json::from_str(SNAPSHOT).unwrap();
    doc.as_object_mut().unwrap().remove("sources");
    doc["digest"] = json!("sha256:4444444444444444");
    *engine.snapshot.lock().unwrap() = doc.to_string();
    let hub = start_classified(
        Some(Arc::clone(&engine)),
        None,
        policy(&[], Some(Class::Public)),
        None,
    )
    .await;
    let r = hub.get(&route("sources")).await;
    assert_eq!(r.status, 502, "{r:?}");
    assert_eq!(r.json()["error"]["code"], "engine_failed");
    // The rules of that snapshot are still read.
    let rule = hub.get(&format!("{}?id=DEP-01", route("rule"))).await;
    assert_eq!(rule.status, 200, "{rule:?}");
}

#[tokio::test]
async fn a_broken_file_list_costs_the_sources_route_but_not_the_rules() {
    let engine = FakeEngine::new();
    let mut doc: Value = serde_json::from_str(SNAPSHOT).unwrap();
    // A file without its count of rules.
    doc["sources"] = json!([{ "etag": "sha256:0000000000000000", "path": "Checklists/DEP.md" }]);
    doc["digest"] = json!("sha256:5555555555555555");
    *engine.snapshot.lock().unwrap() = doc.to_string();
    let hub = start_classified(
        Some(Arc::clone(&engine)),
        None,
        policy(&[], Some(Class::Public)),
        None,
    )
    .await;
    let r = hub.get(&route("sources")).await;
    assert_eq!(r.status, 502, "{r:?}");
    assert_eq!(r.json()["error"]["code"], "engine_failed");
    let rule = hub.get(&format!("{}?id=DEP-01", route("rule"))).await;
    assert_eq!(rule.status, 200, "{rule:?}");
}

/// Only what the loader refuses: no start reads `sources.json` yet, so that the start stops on
/// such a file is tested once the program's `serve` does.
#[test]
fn a_sources_json_with_a_hash_key_does_not_load() {
    let dir = Scratch::new("routes-hash");
    std::fs::write(
        rules_hub::sources::path(&dir),
        r#"{"schema":1,"sources":{"Checklists#2":{"class":"public"}}}"#,
    )
    .unwrap();
    assert!(SourcePolicy::load(&dir).is_err());
    std::fs::write(rules_hub::sources::path(&dir), "{").unwrap();
    assert!(SourcePolicy::load(&dir).is_err());
    std::fs::write(
        rules_hub::sources::path(&dir),
        r#"{"schema":1,"sources":{"Checklists":{"class":"public"}}}"#,
    )
    .unwrap();
    assert_eq!(SourcePolicy::load(&dir).unwrap().len(), 1);
}

// The live routes: the event stream, the rescan and the search.

fn quick() -> rules_hub::store::RefreshOptions {
    rules_hub::store::RefreshOptions {
        debounce: Duration::from_millis(50),
        max_wait: Duration::from_millis(300),
        interval: Duration::from_secs(600),
    }
}

/// A folder to watch, shaped like a vault.
fn watched(name: &str) -> Scratch {
    let dir = Scratch::new(&format!("live-{name}"));
    std::fs::create_dir_all(dir.join("rulesets").join("alpha")).unwrap();
    dir
}

async fn start_watching(engine: &Arc<FakeEngine>, dir: &std::path::Path) -> Hub {
    start_classified(
        Some(Arc::clone(engine)),
        None,
        Arc::default(),
        Some(Watch {
            vault: dir.to_path_buf(),
            refresh: quick(),
        }),
    )
    .await
}

/// The golden snapshot with another etag for DEP-01 and another digest: a change.
fn changed_snapshot() -> String {
    let mut doc: Value = serde_json::from_str(SNAPSHOT).unwrap();
    doc["digest"] = json!("sha256:7777777777777777");
    doc["rev"] = json!("rev:777777");
    let rules = doc["rules"].as_array_mut().unwrap();
    let dep = rules.iter_mut().find(|r| r["id"] == "DEP-01").unwrap();
    dep["etag"] = json!("sha256:1212121212121212");
    doc.to_string()
}

/// An open event stream, read as it comes.
struct Stream {
    socket: tokio::net::TcpStream,
    seen: String,
}

impl Stream {
    async fn open(hub: &Hub) -> Stream {
        let mut socket = tokio::net::TcpStream::connect(hub.server.addr())
            .await
            .unwrap();
        let host = format!("127.0.0.1:{}", hub.server.addr().port());
        let bearer = format!("Bearer {}", hub.token);
        let bytes = appkit_testkit::wire::request(
            "GET",
            &route("events"),
            &[("Host", host.as_str()), ("Authorization", bearer.as_str())],
            b"",
        );
        socket.write_all(&bytes).await.unwrap();
        Stream {
            socket,
            seen: String::new(),
        }
    }

    /// Read until the text holds `needle`; the whole text read so far.
    async fn until(&mut self, needle: &str) -> String {
        let end = std::time::Instant::now() + Duration::from_secs(15);
        while !self.seen.contains(needle) {
            let left = end.saturating_duration_since(std::time::Instant::now());
            let mut buf = [0u8; 4096];
            let n = tokio::time::timeout(left, self.socket.read(&mut buf))
                .await
                .unwrap_or_else(|_| panic!("no {needle:?} in {:?}", self.seen))
                .unwrap();
            assert!(n > 0, "the stream ended before {needle:?}: {:?}", self.seen);
            self.seen.push_str(&String::from_utf8_lossy(&buf[..n]));
        }
        self.seen.clone()
    }

    /// Read until the hub closes the connection (the request says `Connection: close`, so the
    /// end of the stream's body is the end of the connection); the whole text read.
    async fn rest(&mut self) -> String {
        let end = std::time::Instant::now() + Duration::from_secs(15);
        loop {
            let left = end.saturating_duration_since(std::time::Instant::now());
            let mut buf = [0u8; 4096];
            let n = tokio::time::timeout(left, self.socket.read(&mut buf))
                .await
                .unwrap_or_else(|_| panic!("the stream did not end: {:?}", self.seen))
                .unwrap_or(0);
            if n == 0 {
                return self.seen.clone();
            }
            self.seen.push_str(&String::from_utf8_lossy(&buf[..n]));
        }
    }
}

#[tokio::test]
async fn the_stream_says_hello_and_then_what_the_watcher_saw() {
    let engine = FakeEngine::new();
    let dir = watched("fs");
    let hub = start_watching(&engine, &dir).await;
    let mut stream = Stream::open(&hub).await;
    let hello = stream.until("event: hello").await;
    assert!(hello.starts_with("HTTP/1.1 200"), "{hello}");
    assert!(hello.contains("text/event-stream"), "{hello}");
    stream.until("\n\n").await;

    // A rule file changes on disk, and the engine now says something else about it.
    *engine.snapshot.lock().unwrap() = changed_snapshot();
    std::fs::write(dir.join("rulesets/alpha/new.md"), "# x\n").unwrap();
    let text = stream.until("event: rules").await;
    assert!(text.contains("DEP-01"), "{text}");
    assert!(text.contains(r#""cause":"fs""#), "{text}");
}

#[tokio::test]
async fn a_file_that_is_no_rule_wakes_nothing() {
    let engine = FakeEngine::new();
    let dir = watched("quiet");
    let hub = start_watching(&engine, &dir).await;
    let mut stream = Stream::open(&hub).await;
    stream.until("event: hello").await;
    let before = engine.count("snapshot");
    *engine.snapshot.lock().unwrap() = changed_snapshot();
    std::fs::write(dir.join("README.md"), "x").unwrap();
    std::fs::write(dir.join("rulesets/alpha/.new.md.swp"), "x").unwrap();
    tokio::time::sleep(Duration::from_millis(800)).await;
    assert_eq!(engine.count("snapshot"), before, "a scan was woken");
    assert!(!stream.seen.contains("event: rules"), "{}", stream.seen);
}

#[tokio::test]
async fn a_rescan_is_accepted_and_the_stream_tells_what_it_found() {
    let engine = FakeEngine::new();
    let dir = watched("rescan");
    let hub = start_watching(&engine, &dir).await;
    let mut stream = Stream::open(&hub).await;
    stream.until("event: hello").await;

    let anonymous = post(&hub.server, &route("rescan"), None, "{}").await;
    assert_eq!(anonymous.status, 401, "{anonymous:?}");

    *engine.snapshot.lock().unwrap() = changed_snapshot();
    let r = post(&hub.server, &route("rescan"), Some(&hub.token), "{}").await;
    assert_eq!(r.status, 202, "{r:?}");
    let text = stream.until("event: rules").await;
    assert!(text.contains(r#""cause":"rescan""#), "{text}");
}

#[tokio::test]
async fn a_session_has_a_few_streams_and_no_more() {
    let engine = FakeEngine::new();
    let hub = start(&engine).await;
    let mut open = Vec::new();
    for _ in 0..4 {
        let mut s = Stream::open(&hub).await;
        s.until("event: hello").await;
        open.push(s);
    }
    let mut fifth = Stream::open(&hub).await;
    let said = fifth.until("\r\n\r\n").await;
    assert!(said.starts_with("HTTP/1.1 429"), "{said}");
}

/// K22: the session is checked before the hello too. The route waits for the snapshot the stream
/// starts from (a scan of the engine), and the session may be closed meanwhile.
#[tokio::test]
async fn a_stream_whose_session_is_closed_while_it_opens_is_not_greeted() {
    let engine = FakeEngine::new();
    let gate = Arc::new(tokio::sync::Semaphore::new(0));
    *engine.snapshot_gate.lock().unwrap() = Some(Arc::clone(&gate));
    let limits = Limits {
        max_sessions: 2,
        // Without the check before the hello, the next tick ends the stream: after the hello.
        heartbeat: Duration::from_millis(200),
        ..Limits::default()
    };
    let hub = start_limited(
        Some(Arc::clone(&engine)),
        limits,
        Arc::default(),
        None,
        None,
    )
    .await;
    let mut stream = Stream::open(&hub).await;
    let end = std::time::Instant::now() + Duration::from_secs(15);
    while engine.count("snapshot") == 0 {
        assert!(std::time::Instant::now() < end, "no snapshot was asked");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    // Two newer sessions close the one of the stream while it waits for the snapshot.
    session(&hub.server, &hub.control).await;
    session(&hub.server, &hub.control).await;
    let closed = hub.get(&route("snapshot")).await;
    assert_eq!(closed.status, 401, "the session is still open: {closed:?}");
    gate.add_permits(1000);
    let seen = stream.rest().await;
    assert!(seen.starts_with("HTTP/1.1 200"), "{seen}");
    assert!(
        !seen.contains("event: hello"),
        "a closed session was greeted: {seen}"
    );
}

#[tokio::test]
async fn without_an_engine_there_is_no_stream_no_search_and_no_rescan() {
    let hub = start_with(None, None).await;
    for name in ["events", "search"] {
        let path = if name == "search" {
            format!("{}?q=text", route(name))
        } else {
            route(name)
        };
        let r = hub.get(&path).await;
        assert_eq!(r.status, 503, "{name}: {r:?}");
    }
    let r = post(&hub.server, &route("rescan"), Some(&hub.token), "{}").await;
    assert_eq!(r.status, 503, "{r:?}");
}

#[tokio::test]
async fn the_search_finds_the_rules_whose_title_or_text_hold_the_words() {
    let engine = FakeEngine::new();
    let hub = start(&engine).await;
    let path = format!("{}?q=text%20of", route("search"));
    let mut answer = Value::Null;
    for _ in 0..100 {
        let r = hub.get(&path).await;
        assert_eq!(r.status, 200, "{r:?}");
        answer = r.json();
        if answer["status"] == "ready" {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert_eq!(answer["kind"], "hub.search");
    assert_eq!(answer["status"], "ready", "{answer}");
    // The fake knows the text of DEP-01 only ("Text of one."); the other rules are not found by
    // the engine and are left out of the index.
    let ids: Vec<&str> = answer["hits"]
        .as_array()
        .unwrap()
        .iter()
        .map(|h| h["id"].as_str().unwrap())
        .collect();
    assert_eq!(ids, ["DEP-01"], "{answer}");
    assert!(answer["hits"][0]["snippet"]["text"]
        .as_str()
        .unwrap()
        .contains("Text of one"));
    // The title counts, too.
    let by_title = hub.get(&format!("{}?q=one", route("search"))).await.json();
    assert_eq!(by_title["hits"][0]["id"], "DEP-01");
    // `bodies` is not a route: the kit's call is the hub's own.
    let r = hub.get(&route("bodies")).await;
    assert_eq!(r.status, 404, "{r:?}");
}

#[tokio::test]
async fn a_bad_search_is_refused_before_the_index_is_asked() {
    let engine = FakeEngine::new();
    let hub = start(&engine).await;
    for bad in ["", "?q=", "?q=a", "?x=1", "?q=text&q=more", "?q=%ZZ"] {
        let r = hub.get(&format!("{}{bad}", route("search"))).await;
        assert_eq!(r.status, 400, "{bad:?}: {r:?}");
    }
}

#[tokio::test]
async fn the_page_of_a_folder_is_served_to_everybody_and_nothing_else_of_it() {
    let dir = Scratch::new("page");
    std::fs::create_dir_all(dir.join("assets")).unwrap();
    std::fs::write(dir.join("index.html"), "<!doctype html><title>x</title>").unwrap();
    std::fs::write(dir.join("assets").join("app-1.js"), "export {};").unwrap();
    std::fs::write(dir.join("secret.html"), "no").unwrap();
    let control = Secret::generate(TOKEN_BYTES).unwrap();
    let server = rules_hub::start(rules_hub::Start {
        addr: "127.0.0.1:0".parse().unwrap(),
        limits: Limits::default(),
        clock: Arc::new(ManualClock::new()),
        pairing_ttl: Duration::from_secs(60),
        control_secret: control,
        instance: Secret::generate(CODE_BYTES).unwrap(),
        status: ready(),
        engine: None,
        host: None,
        sources: Arc::default(),
        watch: None,
        editor: None,
        login: None,
        ui: rules_hub::ui::UiFiles::load(&dir).unwrap(),
        project: None,
        config_dir: None,
    })
    .await
    .unwrap();
    let page = get(&server, "/", None).await;
    assert_eq!(page.status, 200, "{page:?}");
    assert!(page.body.contains("<title>x</title>"));
    assert_eq!(get(&server, "/assets/app-1.js", None).await.status, 200);
    // Only what the build holds: a file next to it is not there.
    assert_eq!(get(&server, "/secret.html", None).await.status, 404);
    let csp = page.header("content-security-policy").unwrap_or_default();
    assert!(csp.contains("default-src 'none'"), "{csp}");
    assert!(
        csp.contains("script-src 'self'") && !csp.contains("unsafe"),
        "{csp}"
    );
}

// Opening the file of a rule in an editor.

/// An editor that writes down what it was asked to show.
struct Recorder(Mutex<Vec<(std::path::PathBuf, Option<u32>)>>);

impl rules_hub::editor::FileOpener for Recorder {
    fn open(&self, file: &std::path::Path, line: Option<u32>) -> Result<(), String> {
        self.0.lock().unwrap().push((file.to_path_buf(), line));
        Ok(())
    }
}

/// An engine that says where a rule's file is, as the answer of `rules.lua locate`, and keeps the questions.
struct Places {
    answer: Mutex<Result<rules_hub::locate::Located, &'static str>>,
    asked: Mutex<Vec<String>>,
}

impl rules_hub::locate::Locate for Places {
    fn locate<'a>(
        &'a self,
        id: &'a str,
    ) -> appkit_hub::BoxFuture<'a, Result<rules_hub::locate::Located, &'static str>> {
        self.asked.lock().unwrap().push(id.to_owned());
        let answer = self.answer.lock().unwrap().clone();
        Box::pin(async move { answer })
    }
}

struct Opening {
    hub: Hub,
    engine: Arc<FakeEngine>,
    recorder: Arc<Recorder>,
    places: Arc<Places>,
}

impl Opening {
    fn says(&self, file: &std::path::Path, root: Option<&std::path::Path>) {
        *self.places.answer.lock().unwrap() = Ok(rules_hub::locate::Located {
            file: file.to_path_buf(),
            root: root.map(std::path::Path::to_path_buf),
        });
    }

    async fn open(&self, body: &str) -> appkit_testkit::wire::Reply {
        post(
            &self.hub.server,
            &route("open"),
            Some(&self.hub.token),
            body,
        )
        .await
    }

    fn opened(&self) -> Vec<(std::path::PathBuf, Option<u32>)> {
        self.recorder.0.lock().unwrap().clone()
    }
}

async fn start_editing() -> Opening {
    let engine = FakeEngine::new();
    let recorder = Arc::new(Recorder(Mutex::new(Vec::new())));
    let places = Arc::new(Places {
        answer: Mutex::new(Err("not told")),
        asked: Mutex::new(Vec::new()),
    });
    let hub = start_full(
        Some(Arc::clone(&engine)),
        None,
        Arc::default(),
        None,
        Some(rules_hub::Editor {
            locator: Arc::clone(&places) as Arc<dyn rules_hub::locate::Locate>,
            opener: Arc::clone(&recorder) as Arc<dyn rules_hub::editor::FileOpener>,
        }),
    )
    .await;
    Opening {
        hub,
        engine,
        recorder,
        places,
    }
}

/// A rule set with a file in it, somewhere that is nothing like `<vault>/rulesets/<name in the document>`.
fn ruleset(name: &str) -> (Scratch, std::path::PathBuf) {
    let root = Scratch::new(&format!("open-{name}"));
    std::fs::create_dir_all(root.join("sub")).unwrap();
    let file = root.join("sub").join("one.md");
    std::fs::write(&file, "# one\n").unwrap();
    (root, file)
}

#[tokio::test]
async fn the_file_comes_from_the_engine_and_not_from_the_name_in_the_document() {
    let o = start_editing().await;
    // The golden rule says its source is `Checklists/DEP.md`; its file is somewhere that name does not lead to.
    let (root, file) = ruleset("place");
    o.says(&file, Some(root.path()));
    let r = o.open(r#"{"id":"DEP-01"}"#).await;
    assert_eq!(r.status, 200, "{r:?}");
    let seen = o.opened();
    assert_eq!(seen.len(), 1, "{seen:?}");
    assert_eq!(
        seen[0].0.canonicalize().unwrap(),
        file.canonicalize().unwrap()
    );
    assert_eq!(seen[0].1, Some(5), "the line of the golden rule");
    assert_eq!(*o.places.asked.lock().unwrap(), ["DEP-01"]);
}

#[tokio::test]
async fn only_a_rule_of_the_snapshot_is_asked_for() {
    let o = start_editing().await;
    let (root, file) = ruleset("known");
    o.says(&file, Some(root.path()));
    // An id the snapshot does not hold reaches neither the engine nor the locator.
    for body in [r#"{"id":"NOPE-01"}"#, r#"{"id":"../x"}"#] {
        let r = o.open(body).await;
        assert_eq!(r.status, 404, "{body}: {r:?}");
    }
    assert_eq!(
        o.engine.count("rule"),
        0,
        "an id of no rule was asked of the engine"
    );
    assert!(
        o.places.asked.lock().unwrap().is_empty(),
        "an id of no rule was located"
    );
    // The request holds an id and nothing else: a file name from the client is not a thing.
    let r = o.open(r#"{"id":"DEP-01","file":"C:/x"}"#).await;
    assert_eq!(r.status, 400, "{r:?}");
    for bad in [
        "{}",
        "[]",
        "not json",
        r#"{"id":5}"#,
        r#"{"id":"a\u0000b"}"#,
    ] {
        let r = o.open(bad).await;
        assert_eq!(r.status, 400, "{bad}: {r:?}");
    }
    assert!(o.opened().is_empty());
}

#[tokio::test]
async fn a_file_outside_the_ruleset_the_engine_names_is_not_opened() {
    let o = start_editing().await;
    let (root, file) = ruleset("inside");
    let (other_root, other_file) = ruleset("outside");
    let climbing = root
        .join("sub")
        .join("..")
        .join("..")
        .join(other_root.file_name().unwrap())
        .join("sub")
        .join("one.md");
    let cases: Vec<(&str, std::path::PathBuf, Option<&std::path::Path>)> = vec![
        (
            "a file of another rule set",
            other_file.clone(),
            Some(root.path()),
        ),
        ("a climb out of the rule set", climbing, Some(root.path())),
        (
            "a rule set that is not there",
            file.clone(),
            Some(std::path::Path::new("/no/such/set")),
        ),
        ("a folder", root.join("sub"), Some(root.path())),
        (
            "a file that is not there",
            root.join("sub").join("gone.md"),
            Some(root.path()),
        ),
        (
            "a relative place",
            std::path::PathBuf::from("sub/one.md"),
            Some(root.path()),
        ),
        ("no rule set at all", file.clone(), None),
    ];
    for (what, file, set) in cases {
        o.says(&file, set);
        let r = o.open(r#"{"id":"DEP-01"}"#).await;
        assert_eq!(r.status, 400, "{what}: {r:?}");
        assert_eq!(r.json()["error"]["code"], "open_refused", "{what}");
    }
    let _ = other_root;
    assert!(o.opened().is_empty(), "something was opened");
}

#[cfg(unix)]
#[tokio::test]
async fn a_link_in_the_ruleset_does_not_lead_out_of_it() {
    let o = start_editing().await;
    let (root, _) = ruleset("link");
    let (_linked, outside) = ruleset("linked");
    let link = root.join("sub").join("link.md");
    std::os::unix::fs::symlink(&outside, &link).unwrap();
    o.says(&link, Some(root.path()));
    let r = o.open(r#"{"id":"DEP-01"}"#).await;
    assert_eq!(r.status, 400, "{r:?}");
    assert!(o.opened().is_empty());
}

#[tokio::test]
async fn an_engine_that_cannot_say_is_a_bad_gateway_and_opens_nothing() {
    let o = start_editing().await;
    let r = o.open(r#"{"id":"DEP-01"}"#).await;
    assert_eq!(r.status, 502, "{r:?}");
    assert_eq!(r.json()["error"]["code"], "engine_failed");
    assert!(o.opened().is_empty());
}

#[tokio::test]
async fn a_hub_without_an_editor_says_so_and_a_session_is_needed() {
    let engine = FakeEngine::new();
    let hub = start(&engine).await;
    let r = post(
        &hub.server,
        &route("open"),
        Some(&hub.token),
        r#"{"id":"DEP-01"}"#,
    )
    .await;
    assert_eq!(r.status, 404, "{r:?}");
    assert_eq!(r.json()["error"]["code"], "open_unavailable");
    let anonymous = post(&hub.server, &route("open"), None, r#"{"id":"DEP-01"}"#).await;
    assert_eq!(anonymous.status, 401, "{anonymous:?}");
}

// ── the write side: `next_id` and `ops` ─────────────────────────────────────────────────────────

/// Everything has a class on this device: what a write needs.
fn internal() -> Arc<SourcePolicy> {
    Arc::new(SourcePolicy::default().with_default(Some(Class::Internal)))
}

async fn start_writable(engine: &Arc<FakeEngine>) -> Hub {
    start_classified(Some(Arc::clone(engine)), None, internal(), None).await
}

/// A write the engine did: one rule changed.
const WROTE: &str = r#"{"kind":"rules.result","ok":true,"dry_run":false,"results":[{"ok":true,"op":"set","id":"DEP-01","changed_ids":["DEP-01"]}]}"#;
/// A write that changed nothing (a failed operation, a rule that was already so).
const NOTHING: &str = r#"{"kind":"rules.result","ok":false,"dry_run":false,"results":[{"ok":false,"op":"set","id":"DEP-01","changed_ids":[],"error":{"code":"conflict"}}]}"#;

fn ops_body(op: &str, dry_run: bool) -> String {
    json!({
        "schema": 1,
        "client_op_id": "c-1",
        "dry_run": dry_run,
        "ops": [{ "op": op, "id": "DEP-01", "patch": { "severity": "critical" } }],
    })
    .to_string()
}

impl Hub {
    async fn ops(&self, body: &str) -> appkit_testkit::wire::Reply {
        post(&self.server, &route("ops"), Some(&self.token), body).await
    }
}

#[tokio::test]
async fn next_id_asks_for_one_family_or_for_all_and_passes_the_answer_on() {
    let engine = FakeEngine::new();
    let hub = start_writable(&engine).await;
    let all = hub.get(&route("next_id")).await;
    assert_eq!(all.status, 200, "{all:?}");
    assert_eq!(all.body, NEXT_ID.trim_end(), "passed on byte for byte");
    let one = hub.get(&format!("{}?family=DEP", route("next_id"))).await;
    assert_eq!(one.status, 200, "{one:?}");
    assert_eq!(
        engine.params_of("next_id"),
        vec![None, Some(json!({ "family": "DEP" }))]
    );
    // What is not a family never reaches the engine.
    for bad in [
        "family=",
        "family=a%20b",
        "family=A&family=B",
        "x=1",
        "family=AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA",
    ] {
        let r = hub.get(&format!("{}?{bad}", route("next_id"))).await;
        assert_eq!(r.status, 400, "{bad}: {r:?}");
    }
    assert_eq!(engine.count("next_id"), 2);
    assert_eq!(get(&hub.server, &route("next_id"), None).await.status, 401);
}

#[tokio::test]
async fn a_dry_run_is_passed_on_as_it_is_and_reads_nothing_again() {
    let engine = FakeEngine::new();
    let hub = start_writable(&engine).await;
    hub.get(&route("snapshot")).await;
    let body = ops_body("set", true);
    let r = hub.ops(&body).await;
    assert_eq!(r.status, 200, "{r:?}");
    assert_eq!(
        r.body,
        RESULT.trim_end(),
        "the engine's document, unchanged"
    );
    assert_eq!(
        engine.params_of("ops"),
        vec![Some(serde_json::from_str::<Value>(&body).unwrap())]
    );
    assert_eq!(engine.count("snapshot"), 1, "a dry run changes nothing");
}

#[tokio::test]
async fn a_write_that_changed_something_is_followed_by_a_read_before_the_answer() {
    let engine = FakeEngine::new();
    let hub = start_writable(&engine).await;
    hub.get(&route("snapshot")).await;
    assert_eq!(engine.count("snapshot"), 1);
    *engine.ops.lock().unwrap() = WROTE.to_owned();
    let r = hub.ops(&ops_body("set", false)).await;
    assert_eq!(r.status, 200, "{r:?}");
    assert_eq!(r.body, WROTE);
    assert_eq!(engine.count("snapshot"), 2, "the page reads the new state");

    // A write that changed nothing leaves the held snapshot alone.
    *engine.ops.lock().unwrap() = NOTHING.to_owned();
    let r = hub.ops(&ops_body("set", false)).await;
    assert_eq!(r.status, 200, "{r:?}");
    assert_eq!(
        r.body, NOTHING,
        "a conflict is told per operation, not as an error"
    );
    assert_eq!(engine.count("snapshot"), 2);
}

#[tokio::test]
async fn what_is_not_a_request_or_not_served_is_refused_without_a_question_to_the_engine() {
    let engine = FakeEngine::new();
    let hub = start_writable(&engine).await;
    for (name, body) in [
        ("not JSON", "nope".to_owned()),
        ("not an object", "[]".to_owned()),
        (
            "a field of its own",
            r#"{"ops":[{"op":"set"}],"x":1}"#.to_owned(),
        ),
        ("no operation", r#"{"ops":[]}"#.to_owned()),
        ("an unknown operation", ops_body("explode", false)),
        (
            "too many",
            json!({ "ops": vec![json!({ "op": "set" }); 101] }).to_string(),
        ),
        (
            "a bad client_op_id",
            r#"{"client_op_id":"a b","ops":[{"op":"set"}]}"#.to_owned(),
        ),
    ] {
        let r = hub.ops(&body).await;
        assert_eq!(r.status, 400, "{name}: {r:?}");
    }
    assert_eq!(engine.count("ops"), 0);
    let anonymous = post(&hub.server, &route("ops"), None, &ops_body("set", true)).await;
    assert_eq!(anonymous.status, 401, "{anonymous:?}");
}

#[tokio::test]
async fn writes_are_counted_and_dry_runs_are_not() {
    let engine = FakeEngine::new();
    let limits = Limits {
        post_max: 6,
        ..Limits::default()
    };
    let hub = start_limited(Some(Arc::clone(&engine)), limits, internal(), None, None).await;
    // The pairing that made the session was a mutation too: the limit is reached after a few writes.
    let mut done = 0;
    loop {
        let r = hub.ops(&ops_body("set", false)).await;
        if r.status == 429 {
            break;
        }
        assert_eq!(r.status, 200, "{r:?}");
        done += 1;
        assert!(done < 10, "no limit was reached");
    }
    assert!(done >= 1, "the limit left no write at all");
    assert_eq!(engine.count("ops"), done);
    // A dry run asks the engine and costs the writes nothing.
    assert_eq!(hub.ops(&ops_body("set", true)).await.status, 200);
    assert_eq!(engine.count("ops"), done + 1);
}

#[tokio::test]
async fn without_an_engine_a_write_says_so() {
    let hub = start_classified(None, None, internal(), None).await;
    let r = hub.ops(&ops_body("set", false)).await;
    assert_eq!(r.status, 503, "{r:?}");
    assert_eq!(r.json()["error"]["code"], "engine_unavailable");
}

#[tokio::test]
async fn a_rule_set_without_a_class_is_shown_and_never_written() {
    // No sources.json: every rule set is unclassified.
    let engine = FakeEngine::new();
    let hub = start(&engine).await;
    for dry_run in [true, false] {
        let r = hub.ops(&ops_body("set", dry_run)).await;
        assert_eq!(r.status, 403, "{r:?}");
        assert_eq!(r.json()["error"]["code"], "write_refused");
        assert!(!r.body.contains("DEP.md"), "no path in the refusal: {r:?}");
    }
    assert_eq!(engine.count("ops"), 0, "the engine is not asked");
    // It is still read.
    assert_eq!(hub.get(&route("snapshot")).await.status, 200);
}

#[tokio::test]
async fn the_class_decides_per_operation_and_the_whole_request_is_refused_for_one() {
    let engine = FakeEngine::new();
    // Checklists is public, but the file of ERR-100 lies under a key that says nothing, and the default is none.
    let policy =
        Arc::new(SourcePolicy::new([("Checklists/DEP.md".to_owned(), Class::Private)]).unwrap());
    let hub = start_classified(Some(Arc::clone(&engine)), None, policy, None).await;
    // DEP-01 lives in Checklists/DEP.md (private: written, the owner said it), ERR-100 in a file no key names.
    let body = json!({ "ops": [
        { "op": "set", "id": "DEP-01", "patch": { "severity": "critical" } },
        { "op": "set", "id": "ERR-100", "patch": { "severity": "critical" } },
    ]})
    .to_string();
    let r = hub.ops(&body).await;
    assert_eq!(r.status, 403, "{r:?}");
    assert!(r.json()["error"]["message"]
        .as_str()
        .unwrap()
        .starts_with("ops[2]"));
    assert_eq!(engine.count("ops"), 0, "nothing of the request ran");
    let only_first =
        json!({ "ops": [{ "op": "set", "id": "DEP-01", "patch": { "severity": "critical" } }] })
            .to_string();
    assert_eq!(hub.ops(&only_first).await.status, 200);
}

#[tokio::test]
async fn an_id_the_snapshot_does_not_hold_is_a_404_and_new_is_judged_by_its_rule_set() {
    let engine = FakeEngine::new();
    let hub = start_writable(&engine).await;
    for (name, body) in [
        (
            "an unknown id",
            json!({ "ops": [{ "op": "set", "id": "NOPE-01", "patch": {} }] }),
        ),
        // Nothing is retired in the golden snapshot, and a rule that is live cannot be reinstated.
        (
            "reinstate of a live rule",
            json!({ "ops": [{ "op": "reinstate", "id": "DEP-01" }] }),
        ),
    ] {
        let r = hub.ops(&body.to_string()).await;
        assert_eq!(r.status, 404, "{name}: {r:?}");
    }
    let id_missing = json!({ "ops": [{ "op": "set", "patch": {} }] });
    assert_eq!(hub.ops(&id_missing.to_string()).await.status, 400);
    // A new rule goes to the first rule set (the snapshot names it), or to the one it names.
    let new = |ruleset: Option<&str>| {
        let mut op = json!({ "op": "new", "family": "DEP", "title": "t", "severity": "critical" });
        if let Some(name) = ruleset {
            op["ruleset"] = json!(name);
        }
        json!({ "dry_run": true, "ops": [op] }).to_string()
    };
    assert_eq!(hub.ops(&new(None)).await.status, 200);
    assert_eq!(hub.ops(&new(Some("Checklists"))).await.status, 200);
    assert_eq!(
        hub.ops(&new(Some(".."))).await.status,
        404,
        "a name that is no rule set name is not held"
    );
    // The start or the end of a name is not a name.
    assert_eq!(hub.ops(&new(Some("Check"))).await.status, 404);
    assert_eq!(hub.ops(&new(Some("lists"))).await.status, 404);
    let not_a_name = json!({ "ops": [{ "op": "new", "family": "DEP", "ruleset": 7 }] });
    assert_eq!(hub.ops(&not_a_name.to_string()).await.status, 400);

    // A policy that names only another rule set: the first one is not classified, so nothing is written.
    let other = Arc::new(SourcePolicy::new([("elsewhere".to_owned(), Class::Public)]).unwrap());
    let closed = start_classified(Some(Arc::clone(&engine)), None, other, None).await;
    assert_eq!(closed.ops(&new(None)).await.status, 403);
}

#[tokio::test]
async fn a_rule_set_is_named_as_the_snapshot_names_it_not_by_its_last_folder() {
    // The engine finds a rule set by the last folder of its path as well: `Checklists` for `one/Checklists`. The class
    // the hub works out must be the class of the rule set the engine writes into, so only the name of the snapshot counts.
    let engine = FakeEngine::new();
    let renamed = SNAPSHOT
        .trim_end()
        .replace("Checklists/", "one/Checklists/")
        .replace(
            r#""source":{"id":"Checklists"}"#,
            r#""source":{"id":"one/Checklists"}"#,
        );
    *engine.snapshot.lock().unwrap() = renamed;
    let hub = start_writable(&engine).await;
    let new = |ruleset: &str| {
        json!({ "dry_run": true, "ops": [{ "op": "new", "family": "DEP", "title": "t", "severity": "critical", "ruleset": ruleset }] })
            .to_string()
    };
    assert_eq!(hub.ops(&new("one/Checklists")).await.status, 200);
    assert_eq!(hub.ops(&new("Checklists")).await.status, 404);
    assert_eq!(hub.ops(&new("two/Checklists")).await.status, 404);
}

#[tokio::test]
async fn a_rule_set_is_archived_by_the_name_the_snapshot_gives_it_and_never_when_its_files_are_not_known(
) {
    // Named `one/Checklists` by the snapshot: the folder name `Checklists` is only mentioned inside it, and the class
    // worked out for that would not be the class of the folder the engine moves.
    let engine = FakeEngine::new();
    let renamed = SNAPSHOT
        .trim_end()
        .replace("Checklists/", "one/Checklists/")
        .replace(
            r#""source":{"id":"Checklists"}"#,
            r#""source":{"id":"one/Checklists"}"#,
        );
    *engine.snapshot.lock().unwrap() = renamed;
    let hub = start_writable(&engine).await;
    let r = hub
        .ops(&ruleset_body("archive_ruleset", Some("Checklists")))
        .await;
    assert_eq!(r.status, 404, "{r:?}");
    assert_eq!(engine.count("ops"), 0);
    let r = hub
        .ops(&ruleset_body("archive_ruleset", Some("one/Checklists")))
        .await;
    assert_eq!(r.status, 200, "{r:?}");

    // A snapshot that does not say which files it holds: nothing is known of what a folder holds, so none is moved.
    let blind = FakeEngine::new();
    let mut doc: Value = serde_json::from_str(SNAPSHOT).unwrap();
    doc.as_object_mut().unwrap().remove("sources");
    doc["digest"] = json!("sha256:4444444444444444");
    *blind.snapshot.lock().unwrap() = doc.to_string();
    let hub = start_writable(&blind).await;
    let r = hub
        .ops(&ruleset_body("archive_ruleset", Some("anything")))
        .await;
    assert_eq!(r.status, 403, "{r:?}");
    assert_eq!(blind.count("ops"), 0);
    // Making and bringing back need no files.
    assert_eq!(
        hub.ops(&ruleset_body("create_ruleset", Some("x")))
            .await
            .status,
        200
    );
}

// ── the waivers of the project of this device ───────────────────────────────────

async fn start_with_project(engine: &Arc<FakeEngine>, project: Option<&std::path::Path>) -> Hub {
    start_all(
        Some(Arc::clone(engine)),
        Limits::default(),
        // No class for any rule set: a waiver is the project's, not a rule set's.
        Arc::default(),
        None,
        None,
        project.map(std::path::Path::to_path_buf),
        None,
    )
    .await
}

fn waive_body() -> String {
    json!({ "dry_run": true, "ops": [{ "op": "waive", "id": "DEP-01", "reason": "r" }] })
        .to_string()
}

#[tokio::test]
async fn the_waivers_are_those_of_the_project_of_the_device_and_without_one_the_hub_says_so() {
    let engine = FakeEngine::new();
    let none = start_with_project(&engine, None).await;
    let r = none.get(&route("waivers")).await;
    assert_eq!(r.status, 409, "{r:?}");
    assert_eq!(r.json()["error"]["code"], "no_project");
    assert_eq!(engine.count("waivers"), 0, "the engine is not asked");

    let project = std::env::temp_dir();
    let hub = start_with_project(&engine, Some(&project)).await;
    let r = hub.get(&route("waivers")).await;
    assert_eq!(r.status, 200, "{r:?}");
    assert_eq!(r.body, WAIVERS.trim_end(), "passed on byte for byte");
    assert_eq!(
        engine.params_of("waivers"),
        vec![Some(json!({ "project": project.to_string_lossy() }))]
    );
    // The page names no folder: a query is refused.
    let named = hub
        .get(&format!("{}?project=C:/elsewhere", route("waivers")))
        .await;
    assert_eq!(named.status, 400, "{named:?}");
    assert_eq!(get(&hub.server, &route("waivers"), None).await.status, 401);
}

#[tokio::test]
async fn a_waiver_operation_goes_with_the_project_of_the_device_and_never_with_one_of_the_page() {
    let engine = FakeEngine::new();
    let project = std::env::temp_dir();
    let hub = start_with_project(&engine, Some(&project)).await;
    // No class is needed for a waiver, and the project is put in by the hub.
    let r = hub.ops(&waive_body()).await;
    assert_eq!(r.status, 200, "{r:?}");
    let sent = engine.params_of("ops").pop().flatten().unwrap();
    assert_eq!(
        sent["ops"][0]["project"],
        project.to_string_lossy().as_ref()
    );

    // A project of the page is refused, whatever the operation, and the engine is not asked.
    let before = engine.count("ops");
    for op in ["waive", "renew", "remove", "set"] {
        let body =
            json!({ "ops": [{ "op": op, "id": "DEP-01", "project": "C:/elsewhere" }] }).to_string();
        let r = hub.ops(&body).await;
        assert_eq!(r.status, 400, "{op}: {r:?}");
    }
    assert_eq!(engine.count("ops"), before);

    // Without a project of the device a waiver is a 409; a rule operation does not need one.
    let bare = start_with_project(&engine, None).await;
    let r = bare.ops(&waive_body()).await;
    assert_eq!(r.status, 409, "{r:?}");
    assert_eq!(r.json()["error"]["code"], "no_project");
}

// ── the rule sets of the vault ───────────────────────────────────────────────────

fn ruleset_body(op: &str, name: Option<&str>) -> String {
    let mut entry = json!({ "op": op });
    if let Some(name) = name {
        entry["name"] = json!(name);
    }
    json!({ "ops": [entry] }).to_string()
}

/// A rule set made, archived or restored: no rule id, `ruleset` instead.
const MADE: &str = r#"{"kind":"rules.result","ok":true,"dry_run":false,"results":[{"ok":true,"op":"create_ruleset","ruleset":"beta","changed_ids":[],"outcome":"created"}]}"#;
const WOULD: &str = r#"{"kind":"rules.result","ok":true,"dry_run":true,"results":[{"ok":true,"op":"create_ruleset","ruleset":"beta","changed_ids":[],"outcome":"would_create"}]}"#;

#[tokio::test]
async fn a_rule_set_is_made_without_a_class_and_the_hub_reads_the_new_state() {
    let engine = FakeEngine::new();
    // Nothing is classified: making an empty rule set still goes (it is classified before anything is written into it).
    let hub = start(&engine).await;
    hub.get(&route("snapshot")).await;
    assert_eq!(engine.count("snapshot"), 1);
    *engine.ops.lock().unwrap() = MADE.to_owned();
    let r = hub.ops(&ruleset_body("create_ruleset", Some("beta"))).await;
    assert_eq!(r.status, 200, "{r:?}");
    assert_eq!(
        engine.count("snapshot"),
        2,
        "an entry with no rule id is a change too"
    );
    // A dry run changes nothing, and a name is needed.
    *engine.ops.lock().unwrap() = WOULD.to_owned();
    let dry = json!({ "dry_run": true, "ops": [{ "op": "create_ruleset", "name": "beta" }] });
    assert_eq!(hub.ops(&dry.to_string()).await.status, 200);
    assert_eq!(engine.count("snapshot"), 2);
    let r = hub.ops(&ruleset_body("create_ruleset", None)).await;
    assert_eq!(r.status, 400, "{r:?}");
}

#[tokio::test]
async fn a_rule_set_with_rules_goes_to_the_archive_only_if_it_has_a_class_and_goes_alone() {
    let engine = FakeEngine::new();
    let hub = start_writable(&engine).await;
    // Held and classified: passed on.
    let r = hub
        .ops(&ruleset_body("archive_ruleset", Some("Checklists")))
        .await;
    assert_eq!(r.status, 200, "{r:?}");
    // The snapshot does not mention it: it holds no rule, so nothing is protected, and the engine says whether it is there.
    let r = hub
        .ops(&ruleset_body("archive_ruleset", Some("elsewhere")))
        .await;
    assert_eq!(r.status, 200, "{r:?}");
    let before = engine.count("ops");
    // A name is needed, and the operation is alone in its request.
    assert_eq!(
        hub.ops(&ruleset_body("archive_ruleset", None)).await.status,
        400
    );
    let mixed = json!({ "ops": [
        { "op": "archive_ruleset", "name": "Checklists" },
        { "op": "set", "id": "DEP-01", "patch": {} },
    ]});
    assert_eq!(hub.ops(&mixed.to_string()).await.status, 400);
    assert_eq!(engine.count("ops"), before);

    // No class for the rule set: it is not moved.
    let other = Arc::new(SourcePolicy::new([("elsewhere".to_owned(), Class::Public)]).unwrap());
    let closed = start_classified(Some(Arc::clone(&engine)), None, other, None).await;
    let r = closed
        .ops(&ruleset_body("archive_ruleset", Some("Checklists")))
        .await;
    assert_eq!(r.status, 403, "{r:?}");
    assert_eq!(r.json()["error"]["code"], "write_refused");
    assert_eq!(engine.count("ops"), before);
    // Bringing one back needs no class: it is only loaded again.
    let r = closed
        .ops(&ruleset_body("restore_ruleset", Some("Checklists")))
        .await;
    assert_eq!(r.status, 200, "{r:?}");
}

// ── the data classes the page may change ────────────────────────────────────────

fn temp_config(name: &str) -> Scratch {
    Scratch::new(&format!("policy-{name}"))
}

async fn start_with_config(
    engine: &Arc<FakeEngine>,
    policy: SourcePolicy,
    dir: Option<&std::path::Path>,
) -> Hub {
    start_all(
        Some(Arc::clone(engine)),
        Limits::default(),
        Arc::new(policy),
        None,
        None,
        None,
        dir.map(std::path::Path::to_path_buf),
    )
    .await
}

impl Hub {
    async fn set_class(&self, body: Value) -> appkit_testkit::wire::Reply {
        post(
            &self.server,
            &route("sources/class"),
            Some(&self.token),
            &body.to_string(),
        )
        .await
    }
}

#[tokio::test]
async fn without_a_config_folder_the_classes_cannot_be_changed_and_without_a_session_nobody_can() {
    let engine = FakeEngine::new();
    let hub = start_with_config(&engine, SourcePolicy::default(), None).await;
    let r = hub
        .set_class(json!({ "key": "Checklists", "class": "private" }))
        .await;
    assert_eq!(r.status, 409, "{r:?}");
    assert_eq!(r.json()["error"]["code"], "not_configurable");
    let anonymous = post(&hub.server, &route("sources/class"), None, "{}").await;
    assert_eq!(anonymous.status, 401, "{anonymous:?}");
}

#[tokio::test]
async fn a_class_that_holds_back_more_is_written_and_in_force_at_once() {
    let engine = FakeEngine::new();
    let dir = temp_config("tighten");
    let policy = SourcePolicy::new([("Checklists".to_owned(), Class::Internal)]).unwrap();
    let hub = start_with_config(&engine, policy, Some(dir.path())).await;

    let r = hub
        .set_class(json!({ "key": "Checklists", "class": "private" }))
        .await;
    assert_eq!(r.status, 200, "{r:?}");
    assert_eq!(
        std::fs::read_to_string(dir.join("sources.json")).unwrap(),
        "{\n  \"schema\": 1,\n  \"sources\": {\n    \"Checklists\": {\n      \"class\": \"private\"\n    }\n  }\n}\n"
    );
    // The answer of `sources` is the new policy's: the held snapshot did not keep the old one.
    let sources = hub.get(&route("sources")).await.json();
    assert_eq!(sources["keys"][0]["class"], "private", "{sources}");
    assert_eq!(sources["private"].as_array().map(Vec::len), Some(3));
}

#[tokio::test]
async fn a_class_that_releases_more_needs_the_word_of_the_person_and_a_bad_change_writes_nothing() {
    let engine = FakeEngine::new();
    let dir = temp_config("loosen");
    let policy = SourcePolicy::new([("Checklists".to_owned(), Class::Private)]).unwrap();
    let hub = start_with_config(&engine, policy, Some(dir.path())).await;
    let file = dir.join("sources.json");

    // Private to public releases the rule set: refused until the request says it is meant.
    let loosen = json!({ "key": "Checklists", "class": "public" });
    let r = hub.set_class(loosen.clone()).await;
    assert_eq!(r.status, 409, "{r:?}");
    assert_eq!(r.json()["error"]["code"], "confirm_needed");
    assert!(!file.exists(), "nothing was written");
    let sources = hub.get(&route("sources")).await.json();
    assert_eq!(
        sources["keys"][0]["class"], "private",
        "and nothing changed in force"
    );

    let mut meant = loosen;
    meant["confirm_loosen"] = json!(true);
    assert_eq!(hub.set_class(meant).await.status, 200);
    assert!(std::fs::read_to_string(&file)
        .unwrap()
        .contains("\"public\""));

    // What is not a change: a field it does not know, a bad key or class, both targets or none.
    for (name, body) in [
        (
            "an unknown field",
            json!({ "key": "a", "class": "public", "path": "C:/x" }),
        ),
        (
            "a class that is none",
            json!({ "key": "a", "class": "secret" }),
        ),
        (
            "a bad key",
            json!({ "key": "a b", "class": "public", "confirm_loosen": true }),
        ),
        (
            "both targets",
            json!({ "key": "a", "default": true, "class": "private" }),
        ),
        ("no target", json!({ "class": "private" })),
    ] {
        let before = std::fs::read_to_string(&file).unwrap();
        let r = hub.set_class(body).await;
        assert_eq!(r.status, 400, "{name}: {r:?}");
        assert_eq!(std::fs::read_to_string(&file).unwrap(), before, "{name}");
    }
}

#[tokio::test]
async fn a_rule_set_that_gets_a_class_may_be_written_from_then_on() {
    // The class decides whether the page writes (the check before the engine is asked works on the policy in force).
    let engine = FakeEngine::new();
    let dir = temp_config("write");
    let hub = start_with_config(&engine, SourcePolicy::default(), Some(dir.path())).await;
    assert_eq!(hub.ops(&ops_body("set", true)).await.status, 403);
    let r = hub
        .set_class(json!({ "key": "Checklists", "class": "internal", "confirm_loosen": true }))
        .await;
    assert_eq!(r.status, 200, "{r:?}");
    assert_eq!(hub.ops(&ops_body("set", true)).await.status, 200);
    // And taking the key away closes it again (that holds back more: no word needed).
    assert_eq!(
        hub.set_class(json!({ "key": "Checklists", "class": null }))
            .await
            .status,
        200
    );
    assert_eq!(hub.ops(&ops_body("set", true)).await.status, 403);
}
