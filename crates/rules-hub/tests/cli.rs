//! The program as a person runs it: arguments in, text and exit status out; and `serve` with the real
//! engine, a session and the data classes of a config folder.

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::time::{Duration, Instant};

use rules_hub::client;
use serde_json::Value;

mod common;

use common::scratch::Scratch;

fn fixture_vault() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("vault")
}

fn config_dir(name: &str, sources: Option<&str>) -> Scratch {
    let dir = Scratch::new(&format!("cli-{name}"));
    if let Some(text) = sources {
        std::fs::write(dir.join("sources.json"), text).unwrap();
    }
    dir
}

/// The program with a config folder of its own and the fixture vault. Only what a program needs to
/// run and to find Neovim and the engine is handed on; no variable of the person who runs the test
/// that begins with `RULES_` reaches it.
fn command(config: &Path, args: &[&str]) -> Command {
    let mut c = Command::new(env!("CARGO_BIN_EXE_rules-hub"));
    c.args(args).env_clear();
    for (k, v) in std::env::vars_os() {
        let name = k.to_string_lossy().to_ascii_uppercase();
        if !name.starts_with("RULES_") && !name.starts_with("TASKS_") {
            c.env(k, v);
        }
    }
    // No editor is started by a test: the hub writes what it would have opened to this file.
    c.env("RULES_DESKTOP_EDITOR_LOG", config.join("editor.log"))
        .env("RULES_DESKTOP_CONFIG_DIR", config)
        .env("RULES_VAULT", fixture_vault());
    if let Some(engine) = std::env::var_os("RULES_ENGINE_DIR") {
        c.env("RULES_ENGINE_DIR", engine);
    }
    c
}

/// The program to its end. One that has not ended after a minute is a hub that was meant to refuse
/// and runs: it is killed and the test fails, instead of waiting for ever. Its output is read while
/// it runs, so a program that writes a lot cannot block on a full pipe.
fn run(config: &Path, args: &[&str]) -> Output {
    use std::io::Read;
    let mut child = command(config, args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("the program starts");
    let mut out = child.stdout.take().unwrap();
    let mut err = child.stderr.take().unwrap();
    let out = std::thread::spawn(move || {
        let mut v = Vec::new();
        let _ = out.read_to_end(&mut v);
        v
    });
    let err = std::thread::spawn(move || {
        let mut v = Vec::new();
        let _ = err.read_to_end(&mut v);
        v
    });
    let until = Instant::now() + Duration::from_secs(60);
    let status = loop {
        if let Some(status) = child.try_wait().expect("wait") {
            break status;
        }
        if Instant::now() > until {
            let _ = child.kill();
            let _ = child.wait();
            panic!("{args:?} did not end");
        }
        std::thread::sleep(Duration::from_millis(50));
    };
    Output {
        status,
        stdout: out.join().unwrap(),
        stderr: err.join().unwrap(),
    }
}
fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

#[test]
fn the_version_is_the_name_and_the_number() {
    let out = run(&config_dir("version", None), &["--version"]);
    assert!(out.status.success());
    assert_eq!(text(&out.stdout).trim(), rules_hub::version_line());
}

#[test]
fn an_unknown_verb_prints_the_usage_and_fails() {
    let out = run(&config_dir("unknown", None), &["frobnicate"]);
    assert_eq!(out.status.code(), Some(1));
    let err = text(&out.stderr);
    for verb in [
        "serve",
        "start",
        "stop",
        "open",
        "autostart",
        "install",
        "uninstall",
    ] {
        assert!(err.contains(&format!("rules-hub {verb}")), "{verb}: {err}");
    }
    // The usage names this program and not the tasks hub.
    assert!(!err.contains("tasks"), "{err}");
}

#[test]
fn a_bad_port_or_a_stray_argument_is_refused_before_anything_starts() {
    let dir = config_dir("badargs", None);
    for args in [
        &["serve", "--port", "80"][..],
        &["serve", "--port", "x"],
        &["serve", "--bogus"],
        &["stop", "now"],
        &["autostart", "maybe"],
        &["install", "--bogus"],
        &["uninstall", "--bogus"],
        &["open", "--bogus"],
    ] {
        let out = run(&dir, args);
        assert_eq!(out.status.code(), Some(1), "{args:?}");
    }
}

#[test]
fn open_without_a_running_hub_says_so() {
    let out = run(&config_dir("norun", None), &["open"]);
    assert_eq!(out.status.code(), Some(1));
    assert!(text(&out.stderr).contains("no hub is running"));
}

#[test]
fn a_sources_file_that_cannot_be_used_stops_the_start() {
    let dir = config_dir(
        "badsources",
        Some(r#"{"schema":1,"sources":{"alpha":{"class":"publik"}}}"#),
    );
    let out = run(&dir, &["serve", "--port", "40000"]);
    assert_eq!(out.status.code(), Some(1));
    assert!(
        text(&out.stderr).contains("sources.json"),
        "{}",
        text(&out.stderr)
    );
    assert!(!dir.join("hub.json").exists());
}

/// A hub that runs in a child process, stopped when the test ends however it ends.
struct Running {
    child: Child,
    dir: Scratch,
    port: u16,
}

impl Drop for Running {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        // The folder goes with the field, after the hub that held files in it has ended.
    }
}

/// A port nobody listens on and no other test of this run was given: the system may hand the same free
/// port to two callers that bind and drop it one after the other.
fn free_port() -> u16 {
    static GIVEN: std::sync::Mutex<Vec<u16>> = std::sync::Mutex::new(Vec::new());
    let mut given = GIVEN.lock().unwrap();
    loop {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        if !given.contains(&port) {
            given.push(port);
            return port;
        }
    }
}

fn serve(name: &str, sources: Option<&str>) -> Running {
    serve_in(config_dir(name, sources))
}

fn serve_in(dir: Scratch) -> Running {
    let port = free_port();
    let child = command(&dir, &["serve", "--port", &port.to_string()])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("the hub starts");
    let running = Running { child, dir, port };
    let until = Instant::now() + Duration::from_secs(30);
    while !running.dir.join("hub.json").exists() {
        assert!(Instant::now() < until, "the hub did not announce itself");
        std::thread::sleep(Duration::from_millis(100));
    }
    running
}

async fn token(running: &Running) -> String {
    let out = run(&running.dir, &["open"]);
    assert!(out.status.success(), "{}", text(&out.stderr));
    let link = text(&out.stdout);
    let code = link.trim().split_once("#pair=").expect("a pairing link").1;
    let body = format!(r#"{{"code":"{code}"}}"#);
    // The page exchanges the code, so the request carries the origin of the page.
    let origin = format!("http://127.0.0.1:{}", running.port);
    let reply = client::request(
        running.port,
        "POST",
        "/auth/exchange",
        &[("Content-Type", "application/json"), ("Origin", &origin)],
        &body,
    )
    .await
    .expect("exchange");
    assert_eq!(reply.status, 200, "{}", reply.body);
    let doc: Value = serde_json::from_str(&reply.body).unwrap();
    doc["token"].as_str().unwrap().to_owned()
}

async fn get(running: &Running, token: &str, path: &str) -> (u16, Value) {
    let auth = format!("Bearer {token}");
    let reply = client::request(running.port, "GET", path, &[("Authorization", &auth)], "")
        .await
        .expect("request");
    let doc = serde_json::from_str(&reply.body).unwrap_or(Value::Null);
    (reply.status, doc)
}

const CLASSES: &str = r#"{"schema":1,"sources":{
    "alpha":{"class":"public"},"beta":{"class":"internal"},"secret":{"class":"private"}}}"#;

#[tokio::test]
async fn serve_reads_the_fixture_rules_with_the_classes_of_the_config_folder() {
    let hub = serve("serve", Some(CLASSES));
    // Nobody gets in without the pairing: the data routes need a session.
    let anon = client::request(hub.port, "GET", "/api/rules/snapshot", &[], "")
        .await
        .expect("request");
    assert_eq!(anon.status, 401);

    let token = token(&hub).await;
    let (status, hello) = get(&hub, &token, "/api/rules/hello").await;
    assert_eq!(status, 200);
    assert_eq!(hello["engine"]["ready"], true, "{hello}");

    let (status, snap) = get(&hub, &token, "/api/rules/snapshot").await;
    assert_eq!(status, 200, "{snap}");
    let ids: Vec<&str> = snap["rules"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["id"].as_str().unwrap())
        .collect();
    assert_eq!(ids, ["AA-01", "AA-02", "AA-03", "BB-01", "BB-02", "SS-01"]);

    let (status, sources) = get(&hub, &token, "/api/rules/sources").await;
    assert_eq!(status, 200, "{sources}");
    let class_of = |prefix: &str| {
        sources["files"]
            .as_array()
            .unwrap()
            .iter()
            .find(|f| f["path"].as_str().unwrap().starts_with(prefix))
            .map(|f| f["data_class"].as_str().unwrap().to_owned())
    };
    assert_eq!(class_of("alpha/").as_deref(), Some("public"));
    assert_eq!(class_of("beta/").as_deref(), Some("internal"));
    assert_eq!(class_of("secret/").as_deref(), Some("private"));
}

#[tokio::test]
async fn without_a_class_file_every_file_is_unclassified_and_still_shown() {
    let hub = serve("noclasses", None);
    let token = token(&hub).await;
    let (status, sources) = get(&hub, &token, "/api/rules/sources").await;
    assert_eq!(status, 200, "{sources}");
    let files = sources["files"].as_array().unwrap();
    assert_eq!(files.len(), 3);
    assert!(files.iter().all(|f| f["data_class"] == "unclassified"));
}

#[tokio::test]
async fn a_second_serve_in_the_same_config_folder_does_not_start() {
    let hub = serve("twice", Some(CLASSES));
    let out = run(&hub.dir, &["serve", "--port", &free_port().to_string()]);
    assert_eq!(out.status.code(), Some(1));
    assert!(
        text(&out.stderr).contains("already running"),
        "{}",
        text(&out.stderr)
    );
}

/// The login entry and the desktop link of the machine are not the tests': the program leaves them
/// alone when this is set (the kit's switch).
fn without_system_entries(mut c: Command) -> Command {
    c.env("RULES_DESKTOP_NO_LOGIN_SYNC", "1");
    c
}

#[test]
fn stop_without_a_hub_says_so_and_is_no_error() {
    let out = run(&config_dir("stop", None), &["stop"]);
    assert!(out.status.success(), "{}", text(&out.stderr));
    assert!(text(&out.stdout).contains("no hub is running"));
}

#[test]
fn uninstall_purges_the_files_of_the_app_and_only_those() {
    let dir = config_dir("uninstall", Some(CLASSES));
    std::fs::write(dir.join("keep.txt"), "mine").unwrap();
    let out = without_system_entries(command(&dir, &["uninstall", "--purge-settings"]))
        .output()
        .unwrap();
    assert!(out.status.success(), "{}", text(&out.stderr));
    assert!(!dir.join("sources.json").exists(), "sources.json stayed");
    assert!(
        dir.join("keep.txt").exists(),
        "a file of the person was removed"
    );
    let said = text(&out.stdout);
    assert!(said.contains("scripts/uninstall"), "{said}");
    // The script it names is in the repository.
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scripts");
    assert!(root.join("uninstall.ps1").is_file() && root.join("uninstall.sh").is_file());

    // Without the switch for the purge, the settings stay.
    let dir = config_dir("uninstall-keep", Some(CLASSES));
    let out = without_system_entries(command(&dir, &["uninstall"]))
        .output()
        .unwrap();
    assert!(out.status.success(), "{}", text(&out.stderr));
    assert!(dir.join("sources.json").exists());
}

#[test]
fn install_copies_the_program_to_the_install_folder_and_nowhere_else() {
    let dir = config_dir("install", None);
    let target = dir.join("bin");
    let out = without_system_entries(command(&dir, &["install"]))
        .env("RULES_DESKTOP_INSTALL_DIR", &target)
        .output()
        .unwrap();
    assert!(out.status.success(), "{}", text(&out.stderr));
    let installed = target.join(rules_hub::IDENTITY.program_file());
    assert!(installed.is_file(), "{}", text(&out.stdout));
    let version = Command::new(&installed).arg("--version").output().unwrap();
    assert_eq!(text(&version.stdout).trim(), rules_hub::version_line());
}

#[tokio::test]
async fn the_active_vault_of_config_json_is_the_one_that_is_read() {
    // The main vault of the file does not exist; the active one is the fixture. Reading the main
    // vault would leave the engine not ready.
    let dir = config_dir("active", Some(CLASSES));
    let settings = serde_json::json!({
        "vault": dir.join("no-such-vault"),
        "vaults": [{ "name": "fixture", "path": fixture_vault() }],
        "active": "fixture",
    });
    std::fs::write(dir.join("config.json"), settings.to_string()).unwrap();
    let hub = serve_in(dir);
    let token = token(&hub).await;
    let (status, hello) = get(&hub, &token, "/api/rules/hello").await;
    assert_eq!(status, 200);
    assert_eq!(hello["engine"]["ready"], true, "{hello}");
}

#[test]
fn a_port_below_1024_in_config_json_stops_the_start() {
    let dir = config_dir("lowport", None);
    std::fs::write(dir.join("config.json"), r#"{"port":80}"#).unwrap();
    let out = run(&dir, &["serve"]);
    assert_eq!(out.status.code(), Some(1));
    assert!(text(&out.stderr).contains("1024"), "{}", text(&out.stderr));
}

fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap().flatten() {
        let target = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_dir(&entry.path(), &target);
        } else {
            std::fs::copy(entry.path(), target).unwrap();
        }
    }
}

async fn post_json(running: &Running, token: &str, path: &str) -> u16 {
    let auth = format!("Bearer {token}");
    let origin = format!("http://127.0.0.1:{}", running.port);
    client::request(
        running.port,
        "POST",
        path,
        &[
            ("Authorization", &auth),
            ("Origin", &origin),
            ("Content-Type", "application/json"),
        ],
        "{}",
    )
    .await
    .expect("request")
    .status
}

/// Ask until `done` likes the answer; the last answer when it never does within the time.
async fn until(running: &Running, token: &str, path: &str, done: impl Fn(&Value) -> bool) -> Value {
    let end = Instant::now() + Duration::from_secs(30);
    loop {
        let (_, doc) = get(running, token, path).await;
        if done(&doc) || Instant::now() > end {
            return doc;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}

fn has_rule(snapshot: &Value, id: &str) -> bool {
    snapshot["rules"]
        .as_array()
        .is_some_and(|rules| rules.iter().any(|r| r["id"] == id))
}

#[tokio::test]
async fn the_search_reads_the_texts_through_the_real_engine() {
    let hub = serve("search", Some(CLASSES));
    let token = token(&hub).await;
    let doc = until(&hub, &token, "/api/rules/search?q=never%20dropped", |d| {
        d["status"] == "ready"
    })
    .await;
    assert_eq!(doc["status"], "ready", "{doc}");
    assert_eq!(doc["indexed"], 6, "{doc}");
    let ids: Vec<&str> = doc["hits"]
        .as_array()
        .unwrap()
        .iter()
        .map(|h| h["id"].as_str().unwrap())
        .collect();
    assert_eq!(ids, ["AA-01"], "{doc}");
    // The text is a text: the markup in the rule is what the snippet shows, character for character.
    let markup = get(&hub, &token, "/api/rules/search?q=Not%20markup")
        .await
        .1;
    let snippet = markup["hits"][0]["snippet"]["text"].as_str().unwrap();
    assert!(snippet.contains("<b>Not markup</b>"), "{markup}");
}

#[tokio::test]
async fn a_new_rule_file_shows_up_by_itself_and_a_rescan_is_accepted() {
    let vault = Scratch::new("live-vault");
    copy_dir(&fixture_vault(), &vault);
    let dir = config_dir("live", Some(CLASSES));
    std::fs::write(
        dir.join("config.json"),
        serde_json::json!({ "vault": vault.path() }).to_string(),
    )
    .unwrap();
    let hub = serve_in(dir);
    let token = token(&hub).await;
    let snapshot = get(&hub, &token, "/api/rules/snapshot").await.1;
    assert!(!has_rule(&snapshot, "AA-04"));

    let rule = |id: &str, word: &str| {
        format!(
            "# More\n\n#### `{id}` - {word}\n\n```rule\nid = \"{id}\",\nseverity = \"recommended\",\n```\n\nThe word is {word}.\n"
        )
    };
    // The engine does not trust a file younger than two seconds: wait it out before each write.
    std::thread::sleep(Duration::from_millis(2200));
    std::fs::write(
        vault.join("rulesets/alpha/regeln/more.md"),
        rule("AA-04", "zebra"),
    )
    .unwrap();
    // By itself: the watcher saw the file, the refresher read the snapshot again.
    let snapshot = until(&hub, &token, "/api/rules/snapshot", |d| {
        has_rule(d, "AA-04")
    })
    .await;
    assert!(
        has_rule(&snapshot, "AA-04"),
        "the watcher did not see the file"
    );
    // The index follows: the new text is found.
    let found = until(&hub, &token, "/api/rules/search?q=zebra", |d| {
        d["total"].as_u64() == Some(1)
    })
    .await;
    assert_eq!(found["hits"][0]["id"], "AA-04", "{found}");

    // And a rescan is accepted (that it is what reads the snapshot is tested with the fake engine, where no watcher runs).
    std::thread::sleep(Duration::from_millis(2200));
    std::fs::write(
        vault.join("rulesets/beta/regeln/more.md"),
        rule("BB-03", "okapi"),
    )
    .unwrap();
    assert_eq!(post_json(&hub, &token, "/api/rules/rescan").await, 202);
    let snapshot = until(&hub, &token, "/api/rules/snapshot", |d| {
        has_rule(d, "BB-03")
    })
    .await;
    assert!(has_rule(&snapshot, "BB-03"));
}

#[tokio::test]
async fn open_writes_the_file_and_the_line_of_the_rule_it_would_show() {
    let hub = serve("open", Some(CLASSES));
    let token = token(&hub).await;
    let auth = format!("Bearer {token}");
    let origin = format!("http://127.0.0.1:{}", hub.port);
    let post = |body: &'static str| {
        let (auth, origin) = (auth.clone(), origin.clone());
        let port = hub.port;
        async move {
            client::request(
                port,
                "POST",
                "/api/rules/open",
                &[
                    ("Authorization", &auth),
                    ("Origin", &origin),
                    ("Content-Type", "application/json"),
                ],
                body,
            )
            .await
            .expect("request")
            .status
        }
    };
    assert_eq!(post(r#"{"id":"AA-01"}"#).await, 200);
    let logged = std::fs::read_to_string(hub.dir.join("editor.log")).unwrap();
    let line = logged.lines().next().unwrap().replace('\\', "/");
    assert!(
        line.contains("/rulesets/alpha/regeln/alpha.md:"),
        "{logged}"
    );
    // An id the rule sets do not hold opens nothing more.
    assert_eq!(post(r#"{"id":"NOPE-01"}"#).await, 404);
    assert_eq!(
        std::fs::read_to_string(hub.dir.join("editor.log"))
            .unwrap()
            .lines()
            .count(),
        1
    );
}

/// A rule file with one rule in it.
fn write_rule(path: &Path, id: &str, title: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(
        path,
        format!("# x\n\n#### `{id}` - {title}\n\n```rule\nid = \"{id}\",\nseverity = \"recommended\",\n```\n\ntext\n"),
    )
    .unwrap();
}

#[tokio::test]
async fn open_finds_the_file_by_the_place_the_engine_knows_not_by_the_name_in_the_document() {
    // Two rule sets outside the vault with the same last folder: the documents call them one/Checklists and
    // two/Checklists. Under `<vault>/rulesets/one/Checklists` lies a decoy of the first name that is not loaded
    // (the vault names its rule sets itself): a hub that puts the name under `rulesets/` opens the decoy.
    let base = Scratch::new("places");
    let (one, two) = (
        base.join("one").join("Checklists"),
        base.join("two").join("Checklists"),
    );
    write_rule(&one.join("a.md"), "AAA-01", "first");
    write_rule(&two.join("b.md"), "BBB-01", "second");
    // An id that looks like a flag loads like any other and must open like any other.
    write_rule(&one.join("h.md"), "--trust=C:/x", "looks like a flag");
    let vault = base.join("vault");
    write_rule(
        &vault.join("rulesets/one/Checklists/a.md"),
        "ZZZ-01",
        "decoy, not loaded",
    );
    std::fs::write(
        vault.join(".rules.json"),
        serde_json::json!({ "rulesets": [one, two] }).to_string(),
    )
    .unwrap();
    let dir = config_dir(
        "places",
        Some(r#"{"schema":1,"default":"internal","sources":{}}"#),
    );
    std::fs::write(
        dir.join("config.json"),
        serde_json::json!({ "vault": vault }).to_string(),
    )
    .unwrap();
    let hub = serve_in(dir);
    let token = token(&hub).await;
    let snapshot = get(&hub, &token, "/api/rules/snapshot").await.1;
    assert!(
        has_rule(&snapshot, "AAA-01") && has_rule(&snapshot, "BBB-01"),
        "{snapshot}"
    );
    assert!(!has_rule(&snapshot, "ZZZ-01"), "the decoy was loaded");

    let auth = format!("Bearer {token}");
    let origin = format!("http://127.0.0.1:{}", hub.port);
    for (id, want) in [
        ("AAA-01", "one/Checklists/a.md"),
        ("BBB-01", "two/Checklists/b.md"),
        ("--trust=C:/x", "one/Checklists/h.md"),
    ] {
        let body = format!(r#"{{"id":"{id}"}}"#);
        let reply = client::request(
            hub.port,
            "POST",
            "/api/rules/open",
            &[
                ("Authorization", &auth),
                ("Origin", &origin),
                ("Content-Type", "application/json"),
            ],
            &body,
        )
        .await
        .expect("request");
        assert_eq!(reply.status, 200, "{id}: {}", reply.body);
        let logged = std::fs::read_to_string(hub.dir.join("editor.log")).unwrap();
        let last = logged.lines().last().unwrap().replace('\\', "/");
        assert!(last.contains(want), "{id}: opened {last}");
        assert!(
            !last.contains("vault/rulesets"),
            "{id}: the decoy was opened: {last}"
        );
    }
}

/// A rule as a file of its own (`<rule set>/_rules/<FAMILY>/<ID>.md`), the form `rules-ops/1` writes.
fn write_rule_file(set: &Path, id: &str, severity: &str, title: &str) -> PathBuf {
    let family = id.split('-').next().unwrap();
    let file = set.join("_rules").join(family).join(format!("{id}.md"));
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    std::fs::write(
        &file,
        format!(
            "---\nid: {id}\nseverity: {severity}\nfamily: {family}\n---\n\n#### `{id}` — {title}\n\n```rule\nid = \"{id}\",\n```\n\nThe text of the rule.\n\n---\n"
        ),
    )
    .unwrap();
    file
}

/// `POST /api/rules/ops` as the page sends it: a session, the origin of the page, JSON.
async fn post_ops(running: &Running, token: &str, body: &Value) -> (u16, Value) {
    let auth = format!("Bearer {token}");
    let origin = format!("http://127.0.0.1:{}", running.port);
    let reply = client::request(
        running.port,
        "POST",
        "/api/rules/ops",
        &[
            ("Authorization", &auth),
            ("Origin", &origin),
            ("Content-Type", "application/json"),
        ],
        &body.to_string(),
    )
    .await
    .expect("request");
    (
        reply.status,
        serde_json::from_str(&reply.body).unwrap_or(Value::Null),
    )
}

fn severity_of(snapshot: &Value, id: &str) -> Option<String> {
    snapshot["rules"].as_array().and_then(|rules| {
        rules
            .iter()
            .find(|r| r["id"] == id)
            .and_then(|r| r["severity"].as_str().map(str::to_owned))
    })
}

#[tokio::test]
async fn a_rule_is_changed_and_changed_back_through_the_real_engine_and_only_its_line_moves() {
    let base = Scratch::new("write");
    let set = base.join("set");
    let file = write_rule_file(&set, "WW-01", "recommended", "first");
    let other = write_rule_file(&set, "WW-02", "recommended", "second");
    let (before, other_before) = (
        std::fs::read_to_string(&file).unwrap(),
        std::fs::read_to_string(&other).unwrap(),
    );
    let vault = base.join("vault");
    std::fs::create_dir_all(&vault).unwrap();
    std::fs::write(
        vault.join(".rules.json"),
        serde_json::json!({ "rulesets": [set] }).to_string(),
    )
    .unwrap();
    let dir = config_dir(
        "write",
        Some(r#"{"schema":1,"default":"internal","sources":{}}"#),
    );
    std::fs::write(
        dir.join("config.json"),
        serde_json::json!({ "vault": vault }).to_string(),
    )
    .unwrap();
    let hub = serve_in(dir);
    let token = token(&hub).await;
    let snapshot = get(&hub, &token, "/api/rules/snapshot").await.1;
    assert_eq!(
        severity_of(&snapshot, "WW-01").as_deref(),
        Some("recommended")
    );
    let detail = get(&hub, &token, "/api/rules/rule?id=WW-01").await.1;
    assert_eq!(detail["rule"]["etag_scope"], "rule", "{detail}");
    let etag = detail["rule"]["etag"].as_str().unwrap().to_owned();

    // The next id is the engine's to say.
    let next = get(&hub, &token, "/api/rules/next_id?family=WW").await.1;
    assert_eq!(next["items"][0]["next"], "WW-03", "{next}");

    // A dry run says what would happen and writes nothing.
    let set_critical = |dry_run: bool, if_match: &str| {
        serde_json::json!({
            "schema": 1, "client_op_id": "w-1", "dry_run": dry_run,
            "ops": [{ "op": "set", "id": "WW-01", "patch": { "severity": "critical" }, "if_match": if_match }],
        })
    };
    let (status, dry) = post_ops(&hub, &token, &set_critical(true, &etag)).await;
    assert_eq!(status, 200, "{dry}");
    assert_eq!(dry["ok"], true, "{dry}");
    assert_eq!(std::fs::read_to_string(&file).unwrap(), before);

    // A version the person did not see is a conflict, told per operation, and nothing is written.
    let (status, stale) = post_ops(
        &hub,
        &token,
        &set_critical(false, "sha256:0000000000000000"),
    )
    .await;
    assert_eq!(status, 200, "{stale}");
    assert_eq!(stale["ok"], false, "{stale}");
    assert_eq!(stale["results"][0]["error"]["code"], "conflict", "{stale}");
    assert_eq!(std::fs::read_to_string(&file).unwrap(), before);

    // The write: the file changes in the one line, the other rule's file not at all, and the hub has read
    // the new state before it answers.
    let (status, done) = post_ops(&hub, &token, &set_critical(false, &etag)).await;
    assert_eq!(status, 200, "{done}");
    assert_eq!(done["ok"], true, "{done}");
    assert_eq!(
        std::fs::read_to_string(&file).unwrap(),
        before.replace("severity: recommended", "severity: critical")
    );
    assert_eq!(std::fs::read_to_string(&other).unwrap(), other_before);
    let snapshot = get(&hub, &token, "/api/rules/snapshot").await.1;
    assert_eq!(severity_of(&snapshot, "WW-01").as_deref(), Some("critical"));

    // Back: the engine's own inverse, as a request of its own.
    let inverse = done["results"][0]["inverse"].clone();
    assert!(inverse.is_object(), "{done}");
    let undo = serde_json::json!({ "schema": 1, "client_op_id": "w-2", "ops": [inverse] });
    let (status, undone) = post_ops(&hub, &token, &undo).await;
    assert_eq!(status, 200, "{undone}");
    assert_eq!(undone["ok"], true, "{undone}");
    assert_eq!(std::fs::read_to_string(&file).unwrap(), before);

    // A waiver needs the project of the device; there is none in this config, so the hub says so.
    let waive = serde_json::json!({ "ops": [{ "op": "waive", "id": "WW-01", "reason": "x" }] });
    assert_eq!(post_ops(&hub, &token, &waive).await.0, 409);
}

#[tokio::test]
async fn waivers_are_made_renewed_and_removed_through_the_real_engine_in_the_project_of_the_device()
{
    let base = Scratch::new("waivers");
    let set = base.join("set");
    write_rule_file(&set, "WW-01", "recommended", "first");
    let project = base.join("project");
    std::fs::create_dir_all(&project).unwrap();
    let vault = base.join("vault");
    std::fs::create_dir_all(&vault).unwrap();
    std::fs::write(
        vault.join(".rules.json"),
        serde_json::json!({ "rulesets": [set] }).to_string(),
    )
    .unwrap();
    let dir = config_dir(
        "waivers",
        Some(r#"{"schema":1,"default":"internal","sources":{}}"#),
    );
    // The project is the person's word about this device, in config.json: the page names no folder.
    std::fs::write(
        dir.join("config.json"),
        serde_json::json!({ "vault": vault, "project": project }).to_string(),
    )
    .unwrap();
    let file = project.join(".rules-waivers.json");
    let hub = serve_in(dir);
    let token = token(&hub).await;

    let none = get(&hub, &token, "/api/rules/waivers").await;
    assert_eq!(none.0, 200, "{}", none.1);
    assert_eq!(none.1["exists"], false);
    assert_eq!(none.1["items"].as_array().map(Vec::len), Some(0));
    let empty_etag = none.1["etag"].as_str().unwrap().to_owned();

    // A dry run says what would be written and writes nothing.
    let waive = |dry_run: bool, if_match: &str| {
        serde_json::json!({
            "schema": 1, "client_op_id": "v-1", "dry_run": dry_run,
            "ops": [{ "op": "waive", "id": "WW-01", "reason": "legacy", "expires": "2099-12-31", "if_match": if_match }],
        })
    };
    let (status, dry) = post_ops(&hub, &token, &waive(true, &empty_etag)).await;
    assert_eq!((status, dry["ok"].as_bool()), (200, Some(true)), "{dry}");
    assert!(!file.exists());

    // The waiver: the file of the project is written, canonically, and the page reads it back.
    let (status, done) = post_ops(&hub, &token, &waive(false, &empty_etag)).await;
    assert_eq!((status, done["ok"].as_bool()), (200, Some(true)), "{done}");
    let written = std::fs::read_to_string(&file).unwrap();
    assert_eq!(
        written,
        "{\n  \"WW-01\": {\n    \"expires\": \"2099-12-31\",\n    \"reason\": \"legacy\"\n  }\n}\n"
    );
    let read = get(&hub, &token, "/api/rules/waivers").await.1;
    assert_eq!(read["items"][0]["id"], "WW-01");
    assert_eq!(read["items"][0]["expired"], false);
    assert_eq!(read["items"][0]["known"], true);
    let etag = read["etag"].as_str().unwrap().to_owned();

    // Renew without an end: the form is the short one; a stale tag is a conflict and writes nothing.
    let renew = |if_match: &str| serde_json::json!({ "ops": [{ "op": "renew", "id": "WW-01", "no_expiry": true, "if_match": if_match }] });
    let (_, stale) = post_ops(&hub, &token, &renew("sha256:0000000000000000")).await;
    assert_eq!(stale["results"][0]["error"]["code"], "conflict", "{stale}");
    assert_eq!(std::fs::read_to_string(&file).unwrap(), written);
    let (_, renewed) = post_ops(&hub, &token, &renew(&etag)).await;
    assert_eq!(renewed["ok"], true, "{renewed}");
    assert_eq!(
        std::fs::read_to_string(&file).unwrap(),
        "{\n  \"WW-01\": \"legacy\"\n}\n"
    );

    // The engine's way back puts the end date back, byte for byte.
    let inverse = renewed["results"][0]["inverse"].clone();
    let (_, undone) = post_ops(&hub, &token, &serde_json::json!({ "ops": [inverse] })).await;
    assert_eq!(undone["ok"], true, "{undone}");
    assert_eq!(std::fs::read_to_string(&file).unwrap(), written);

    // Remove: the file keeps an empty object.
    let etag = get(&hub, &token, "/api/rules/waivers").await.1["etag"]
        .as_str()
        .unwrap()
        .to_owned();
    let remove =
        serde_json::json!({ "ops": [{ "op": "remove", "id": "WW-01", "if_match": etag }] });
    let (_, removed) = post_ops(&hub, &token, &remove).await;
    assert_eq!(removed["ok"], true, "{removed}");
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "{}\n");

    // The page cannot point the write at another folder: a project of its own is refused and nothing is written.
    let elsewhere = base.join("elsewhere");
    std::fs::create_dir_all(&elsewhere).unwrap();
    let hijack = serde_json::json!({ "ops": [
        { "op": "waive", "id": "WW-01", "reason": "x", "project": elsewhere }
    ] });
    assert_eq!(post_ops(&hub, &token, &hijack).await.0, 400);
    assert!(!elsewhere.join(".rules-waivers.json").exists());
}

#[tokio::test]
async fn a_rule_set_is_made_moved_to_the_archive_and_brought_back_through_the_real_engine() {
    let base = Scratch::new("rulesets");
    let vault = base.join("vault");
    let sets = vault.join("rulesets");
    let alpha = write_rule_file(&sets.join("alpha"), "AA-01", "recommended", "first");
    let beta = write_rule_file(&sets.join("beta"), "BB-01", "recommended", "second");
    let beta_text = std::fs::read_to_string(&beta).unwrap();
    let dir = config_dir(
        "rulesets",
        Some(r#"{"schema":1,"default":"internal","sources":{}}"#),
    );
    std::fs::write(
        dir.join("config.json"),
        serde_json::json!({ "vault": vault }).to_string(),
    )
    .unwrap();
    let hub = serve_in(dir);
    let token = token(&hub).await;
    let held = until(&hub, &token, "/api/rules/snapshot", |s| {
        has_rule(s, "AA-01") && has_rule(s, "BB-01")
    })
    .await;
    assert!(has_rule(&held, "BB-01"), "{held}");
    let op = |name: &str, verb: &str, dry_run: bool| serde_json::json!({ "schema": 1, "dry_run": dry_run, "ops": [{ "op": verb, "name": name }] });

    // A dry run moves nothing.
    let (status, dry) = post_ops(&hub, &token, &op("beta", "archive_ruleset", true)).await;
    assert_eq!((status, dry["ok"].as_bool()), (200, Some(true)), "{dry}");
    assert!(beta.exists());

    // Archived: the folder moves, nothing is deleted, and the rules of the rule set leave the snapshot by themselves.
    let (status, archived) = post_ops(&hub, &token, &op("beta", "archive_ruleset", false)).await;
    assert_eq!(
        (status, archived["ok"].as_bool()),
        (200, Some(true)),
        "{archived}"
    );
    assert!(!beta.exists());
    let moved = vault.join("archive/rulesets/beta/_rules/BB/BB-01.md");
    assert_eq!(std::fs::read_to_string(&moved).unwrap(), beta_text);
    let after = get(&hub, &token, "/api/rules/snapshot").await.1;
    assert!(!has_rule(&after, "BB-01") && has_rule(&after, "AA-01"));
    // The archive is told by the engine, folder names only.
    let listed = get(&hub, &token, "/api/rules/rulesets").await.1;
    assert_eq!(listed["archived"], serde_json::json!(["beta"]), "{listed}");
    assert_eq!(listed["items"].as_array().map(Vec::len), Some(1));
    assert!(!listed.to_string().contains("archive/"));

    // The last rule set is not archived.
    let (_, last) = post_ops(&hub, &token, &op("alpha", "archive_ruleset", false)).await;
    assert_eq!(last["results"][0]["error"]["code"], "invalid_argument");
    assert!(alpha.exists());

    // The engine's way back brings every byte back.
    let inverse = archived["results"][0]["inverse"].clone();
    let (_, restored) = post_ops(&hub, &token, &serde_json::json!({ "ops": [inverse] })).await;
    assert_eq!(restored["ok"], true, "{restored}");
    assert_eq!(std::fs::read_to_string(&beta).unwrap(), beta_text);
    assert!(!vault.join("archive/rulesets/beta").exists());
    let back = get(&hub, &token, "/api/rules/snapshot").await.1;
    assert!(has_rule(&back, "BB-01"));

    // A new rule set is an empty folder pair; a name that is not one is refused and makes nothing.
    let (_, made) = post_ops(&hub, &token, &op("gamma", "create_ruleset", false)).await;
    assert_eq!(made["ok"], true, "{made}");
    assert!(sets.join("gamma/_rules/.gitkeep").exists());
    assert!(sets.join("gamma/_layouts/.gitkeep").exists());
    // An empty rule set has no rule to show it by: the list of rule sets does.
    let listed = get(&hub, &token, "/api/rules/rulesets").await.1;
    let folders: Vec<_> = listed["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["folder"].as_str().unwrap().to_owned())
        .collect();
    assert_eq!(folders, ["alpha", "beta", "gamma"], "{listed}");
    let (_, bad) = post_ops(&hub, &token, &op("../x", "create_ruleset", false)).await;
    assert_eq!(bad["results"][0]["error"]["code"], "invalid_argument");
    assert!(!vault.join("x").exists());
}

// ── a hub in the background ends itself when it is left alone ──────────────────────────────────

/// Stops the background hub of a config folder when the test ends however it ends (the hub is a process of its
/// own, started by `start`, not a child of the test).
struct Background<'a>(&'a Path);

impl Drop for Background<'_> {
    fn drop(&mut self) {
        let _ = run(self.0, &["stop"]);
    }
}

/// `start` with a short time alone (`RULES_DESKTOP_ALONE_SECS`), on a port of its own, with no login entry of
/// the machine asked.
fn start_in_background(config: &Path, port: u16, alone_secs: u32) {
    let mut c = without_system_entries(command(config, &["start"]));
    let started = c
        .env("RULES_DESKTOP_PORT", port.to_string())
        .env("RULES_DESKTOP_ALONE_SECS", alone_secs.to_string())
        .output()
        .unwrap();
    assert!(started.status.success(), "{}", text(&started.stderr));
    assert!(
        text(&started.stdout).contains("runs in the background"),
        "{started:?}"
    );
}

/// Whether a hub answers on the port.
fn answers(port: u16) -> bool {
    let addr = std::net::SocketAddr::from(([127, 0, 0, 1], port));
    std::net::TcpStream::connect_timeout(&addr, Duration::from_secs(2)).is_ok()
}

/// Wait until the hub of this config folder is gone: no record, nothing on the port.
fn gone_within(config: &Path, port: u16, limit: Duration) -> bool {
    let deadline = Instant::now() + limit;
    while Instant::now() < deadline {
        if !config.join("hub.json").exists() && !answers(port) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    false
}

async fn app_of(port: u16) -> Value {
    let reply = client::request(port, "GET", "/api/rules/hello", &[], "")
        .await
        .expect("hello");
    serde_json::from_str::<Value>(&reply.body).unwrap_or(Value::Null)["app"].clone()
}

/// A hub that was started for a window nobody opened does not stay behind: it has no window, so nobody would ever
/// see or stop it.
#[test]
fn a_hub_in_the_background_that_nobody_opens_ends_itself() {
    let config = config_dir("alone", None);
    let port = free_port();
    let _guard = Background(&config);
    // Ten seconds, not two: `start` has to see the hub answer while it lives, also on a machine under load.
    start_in_background(&config, port, 10);

    assert!(
        gone_within(&config, port, Duration::from_secs(40)),
        "the hub still runs: {}",
        std::fs::read_to_string(config.join("hub.log")).unwrap_or_default()
    );
    // It left on its own terms: its record and its secret are gone, and it said why in its log.
    assert!(!config.join("control.secret").exists());
    let log = std::fs::read_to_string(config.join("hub.log")).unwrap();
    assert!(log.contains("no page is connected"), "{log}");
    assert!(text(&run(&config, &["stop"]).stdout).contains("no hub is running"));
}

/// The hub stays as long as a page holds a connection with its session, and goes once that page is closed.
#[tokio::test]
async fn a_hub_in_the_background_stays_while_a_page_is_connected_and_ends_after_it_closed() {
    use std::io::{Read, Write};

    let config = config_dir("page", None);
    let port = free_port();
    let _guard = Background(&config);
    let alone: u32 = 8;
    start_in_background(&config, port, alone);

    // The page: a pairing link, its session, and a connection that stays open (as the event stream does).
    let out = run(&config, &["open"]);
    assert!(out.status.success(), "{out:?}");
    let link = text(&out.stdout);
    let code = link
        .trim()
        .split_once("#pair=")
        .expect("a pairing link")
        .1
        .to_owned();
    let origin = format!("http://127.0.0.1:{port}");
    let body = format!(r#"{{"code":"{code}"}}"#);
    let headers = [
        ("Content-Type", "application/json"),
        ("Origin", origin.as_str()),
    ];
    let paired = client::request(port, "POST", "/auth/exchange", &headers, &body)
        .await
        .unwrap();
    assert_eq!(paired.status, 200, "{}", paired.body);
    let token = serde_json::from_str::<Value>(&paired.body).unwrap()["token"]
        .as_str()
        .unwrap()
        .to_owned();
    let mut page = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
    page.set_read_timeout(Some(Duration::from_secs(10)))
        .unwrap();
    // The event stream, not a request that is over: an idle connection between two requests is closed by the hub
    // after ten seconds, and the test would then not hang on this page at all.
    write!(
        page,
        "GET /api/rules/events HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nAuthorization: Bearer {token}\r\n\r\n"
    )
    .unwrap();
    let mut head = [0u8; 12];
    page.read_exact(&mut head).unwrap();
    assert_eq!(&head, b"HTTP/1.1 200", "the stream is open");

    // More than twice the time alone, and past the ten seconds an idle connection gets: the stream is still open,
    // so the hub is too.
    tokio::time::sleep(Duration::from_secs(u64::from(alone) * 2 + 6)).await;
    assert_eq!(app_of(port).await, "rules-desktop");
    assert!(config.join("hub.json").exists());

    drop(page);
    assert!(
        gone_within(&config, port, Duration::from_secs(u64::from(alone) + 40)),
        "the page is closed and the hub still runs"
    );
}

/// An order to stay is the person's: `keep_running` in the settings keeps a hub in the background without a page.
/// A hub in a terminal is never left alone either: the person there ends it.
#[tokio::test]
async fn a_hub_with_an_order_to_stay_and_a_hub_in_a_terminal_do_not_end_themselves() {
    let config = config_dir("keep", None);
    let port = free_port();
    let _guard = Background(&config);
    std::fs::write(config.join("config.json"), r#"{"keep_running":true}"#).unwrap();
    start_in_background(&config, port, 1);

    let terminal = config_dir("terminal", None);
    let terminal_port = free_port();
    let child = without_system_entries(command(
        &terminal,
        &["serve", "--port", &terminal_port.to_string()],
    ))
    .env("RULES_DESKTOP_ALONE_SECS", "1")
    .stdout(Stdio::null())
    .stderr(Stdio::null())
    .spawn()
    .expect("rules-hub starts");
    let _hub = Running {
        child,
        dir: terminal,
        port: terminal_port,
    };
    let deadline = Instant::now() + Duration::from_secs(20);
    while !_hub.dir.join("hub.json").exists() {
        assert!(Instant::now() < deadline, "the hub never wrote hub.json");
        std::thread::sleep(Duration::from_millis(50));
    }

    tokio::time::sleep(Duration::from_secs(6)).await;
    assert_eq!(app_of(port).await, "rules-desktop");
    assert_eq!(app_of(terminal_port).await, "rules-desktop");
}
