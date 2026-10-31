//! The commands of the program: `serve` and `open`, and the launcher of the browser.
//!
//! They live in the library so that their parts can be tested; `main.rs` only reads the arguments.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use crate::settings::RulesSettings;
use appkit_core::authn::{Secret, SystemClock, CODE_BYTES, TOKEN_BYTES};
use appkit_core::registry::{self, HubRecord};
use appkit_hub::config::FileConfig;
use appkit_hub::discovery::AppInputs;
use appkit_hub::env::Env;
use appkit_hub::{HubStatus, Limits};
use serde_json::Value;

use crate::editor::TerminalOpener;
use crate::engine::{cli_config, probe, CliEngine, Engine, DISCOVERY};
use crate::locate::ProcessLocator;
use crate::sources::SourcePolicy;
use crate::store::RefreshOptions;
use crate::ui::UiFiles;
use crate::{client, Editor, Start, Watch, IDENTITY};

/// What to do when the hub is started.
#[derive(Debug, Clone, Default)]
pub struct ServeOptions {
    /// Overrides the settings file and the environment.
    pub port: Option<u16>,
}

/// The folder for `hub.json`, `control.secret`, `config.json` and `sources.json`.
fn app_dir(env: &Env) -> Result<PathBuf, String> {
    appkit_hub::instance::config_folder(env, &IDENTITY)
}

/// The port to try first: the option, the environment, the settings file, then the default.
fn wanted_port(
    opts: &ServeOptions,
    file: &FileConfig<RulesSettings>,
    env: &Env,
) -> Result<u16, String> {
    if let Some(p) = opts.port {
        return Ok(p);
    }
    let port_var = IDENTITY.var("PORT");
    if let Some(v) = env.get(&port_var) {
        let text = v.to_string_lossy();
        return text
            .parse::<u16>()
            .ok()
            .filter(|p| *p >= 1024)
            .ok_or_else(|| format!("{port_var} is not a port from 1024 to 65535: {text:?}"));
    }
    match file.port {
        Some(p) if p < 1024 => Err(format!(
            "port in config.json is not a port from 1024 to 65535: {p}"
        )),
        Some(p) => Ok(p),
        None => Ok(IDENTITY.default_port),
    }
}

/// Is a hub of this app running according to `hub.json`? It is only believed when the process on
/// that port says the same instance.
async fn running_hub(dir: &std::path::Path) -> Option<HubRecord> {
    appkit_hub::instance::running_hub(dir, &IDENTITY).await
}

/// Wait for Ctrl-C, or for SIGTERM where there is one.
async fn wait_for_stop() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{signal, SignalKind};
        if let Ok(mut term) = signal(SignalKind::terminate()) {
            tokio::select! {
                _ = tokio::signal::ctrl_c() => {}
                _ = term.recv() => {}
            }
            return;
        }
    }
    let _ = tokio::signal::ctrl_c().await;
}

/// What the start found out about the engine.
pub struct EngineState {
    pub status: HubStatus,
    /// Only when the engine answered `hello` as this app needs.
    pub engine: Option<Arc<dyn Engine>>,
    /// The same engine, asked whether its persistent host runs (the texts of the search are read only then).
    pub host: Option<Arc<dyn crate::bodies::HostProbe>>,
    /// A longer text for the terminal of whoever started the hub (it may hold paths; `hello` gets
    /// only the short code of `status`).
    pub note: Option<String>,
    /// The vault to watch, once the engine is known to be there.
    pub vault: Option<PathBuf>,
    /// The Neovim that was found, for opening a file in it.
    pub nvim: Option<PathBuf>,
    /// How the engine is started, for the questions that are not documents (where a rule's file is).
    pub locate_config: Option<crate::engine::CliEngineConfig>,
}

/// Find Neovim, the engine and the vault, start the engine and ask it `hello`.
pub async fn engine_state(file: &FileConfig<RulesSettings>, env: &Env) -> EngineState {
    let failed = |code: &'static str, note: String| EngineState {
        status: HubStatus {
            engine_ready: false,
            problem: Some(code),
            capabilities: Vec::new(),
        },
        engine: None,
        host: None,
        note: Some(note),
        vault: None,
        nvim: None,
        locate_config: None,
    };
    let inputs = AppInputs {
        lib_dir: None,
        extra_areas: &[],
    };
    let setup = match appkit_hub::discovery::discover(&DISCOVERY, file, inputs, env).await {
        Ok(setup) => setup,
        Err(problem) => return failed(problem.code(), problem.to_string()),
    };
    let cfg = cli_config(&setup, env.clone());
    let locate_config = Some(cfg.clone());
    // A persistent engine process answers in milliseconds where a process per question needs a
    // second; `RULES_HUB_ENGINE=process` asks for the one-process-per-question way.
    let one_process = env
        .get("RULES_HUB_ENGINE")
        .is_some_and(|v| v.to_string_lossy() == "process");
    let engine = if one_process {
        CliEngine::new(cfg)
    } else {
        CliEngine::with_host(cfg)
    };
    let engine = Arc::new(engine);
    match probe(engine.as_ref()).await {
        Ok(info) => EngineState {
            status: HubStatus {
                engine_ready: true,
                problem: None,
                capabilities: info.capabilities,
            },
            engine: Some(Arc::clone(&engine) as Arc<dyn Engine>),
            // One process per question has no host: the texts of the search are not read (a process for each rule).
            host: Some(if one_process {
                Arc::new(crate::bodies::NoHost) as Arc<dyn crate::bodies::HostProbe>
            } else {
                engine as Arc<dyn crate::bodies::HostProbe>
            }),
            vault: Some(setup.vault.value.clone()),
            nvim: Some(setup.nvim.value.clone()),
            locate_config,
            note: setup
                .lib_problem
                .as_ref()
                .map(|p| format!("note: {p} (the engine may still find it)")),
        },
        Err(problem) => failed(problem.code(), problem.to_string()),
    }
}

/// The project of the waivers: `RULES_DESKTOP_PROJECT`, else `project` of `config.json`. One that cannot be used stops the
/// start (a typo must not turn into waivers written somewhere else).
fn project_of(
    file: &FileConfig<RulesSettings>,
    env: &Env,
) -> Result<Option<std::path::PathBuf>, String> {
    let project = env
        .path(&crate::IDENTITY.var("PROJECT"))
        .or_else(|| file.app.project.clone());
    if let Some(path) = &project {
        crate::settings::check_project(path).map_err(|why| format!("project: {why}"))?;
    }
    Ok(project)
}

/// Run the server until Ctrl-C.
///
/// # Errors
///
/// A text for the person at the terminal: what is missing or in the way.
pub async fn serve(opts: ServeOptions) -> Result<(), String> {
    let env = Env::from_process();
    let dir = app_dir(&env)?;
    let file: FileConfig<RulesSettings> =
        appkit_hub::config::load_file(&dir).map_err(|e| e.to_string())?;
    let want = wanted_port(&opts, &file, &env)?;

    if let Some(other) = running_hub(&dir).await {
        return Err(format!(
            "a hub is already running (port {}, process {})",
            other.port, other.pid
        ));
    }

    // A file that cannot be read stops the start: a typo must not turn into "everything
    // unclassified" without anyone seeing it.
    let sources = Arc::new(SourcePolicy::load(&dir).map_err(|e| e.to_string())?);
    if sources.is_empty() && sources.default_class().is_none() {
        eprintln!(
            "note: {} has no data class for any rule set: every rule is shown as unclassified",
            crate::sources::path(&dir).display()
        );
    }

    let ui = match env.path("RULES_HUB_UI_DIR") {
        Some(dir) => match UiFiles::load(&dir) {
            Ok(files) => files,
            Err(e) => {
                eprintln!(
                    "note: the interface in {} cannot be used: {e}",
                    dir.display()
                );
                UiFiles::embedded()
            }
        },
        None => UiFiles::embedded(),
    };

    let project = project_of(&file, &env)?;
    let state = engine_state(&file.effective(), &env).await;
    if let Some(note) = &state.note {
        eprintln!("{note}");
    }
    let status = state.status.clone();

    let control = Secret::generate(TOKEN_BYTES).map_err(|e| e.to_string())?;
    let instance = Secret::generate(CODE_BYTES).map_err(|e| e.to_string())?;
    let make = |port: u16| Start {
        addr: std::net::SocketAddr::from(([127, 0, 0, 1], port)),
        limits: Limits {
            snapshot_max_age: state.engine.as_ref().map(|_| Duration::from_secs(120)),
            ..Limits::default()
        },
        clock: Arc::new(SystemClock::new()),
        pairing_ttl: Duration::from_secs(60),
        control_secret: control.clone(),
        instance: instance.clone(),
        status: status.clone(),
        engine: state.engine.clone(),
        host: state.host.clone(),
        sources: Arc::clone(&sources),
        editor: state
            .locate_config
            .clone()
            .zip(state.nvim.clone())
            .map(|(cfg, nvim)| Editor {
                locator: Arc::new(ProcessLocator::new(cfg)),
                opener: Arc::new(TerminalOpener::new(nvim, env.clone())),
            }),
        ui: ui.clone(),
        project: project.clone(),
        config_dir: Some(dir.clone()),
        watch: state.vault.clone().map(|vault| Watch {
            vault,
            refresh: RefreshOptions::default(),
        }),
    };
    // The port is part of the origin, so the wanted one is tried first. When it is taken the server
    // still starts, on another port, and says so.
    let running = match crate::start(make(want)).await {
        Ok(r) => r,
        Err(e) if e.kind() == std::io::ErrorKind::AddrInUse => {
            if appkit_hub::instance::hello_of(want, &IDENTITY)
                .await
                .is_some()
            {
                return Err(format!("a hub is already running on port {want}"));
            }
            eprintln!("note: port {want} is in use, using a free port; the origin changes");
            crate::start(make(0)).await.map_err(|e| e.to_string())?
        }
        Err(e) => return Err(e.to_string()),
    };
    let port = running.addr().port();

    let record = HubRecord {
        app: IDENTITY.app.to_owned(),
        port,
        pid: std::process::id(),
        version: crate::version().to_owned(),
        instance: running.instance().to_owned(),
    };
    let written = registry::write_secret(&dir, &control.to_hex())
        .and_then(|()| registry::write_record(&dir, &record));
    if let Err(e) = written {
        registry::remove_secret(&dir);
        registry::remove_record(&dir);
        running.shutdown().await;
        return Err(e.to_string());
    }

    println!(
        "{} hub {} on http://127.0.0.1:{port}/",
        IDENTITY.app,
        crate::version()
    );
    println!(
        "run `{} open` for a pairing link; Ctrl-C stops the hub",
        IDENTITY.binary
    );

    wait_for_stop().await;
    registry::remove_record(&dir);
    registry::remove_secret(&dir);
    running.shutdown().await;
    Ok(())
}

/// Ask the running hub for a pairing code and return the link for it.
///
/// # Errors
///
/// A text for the person at the terminal.
pub async fn open() -> Result<String, String> {
    let env = Env::from_process();
    let dir = app_dir(&env)?;
    let record = registry::read_record(&dir)
        .map_err(|e| e.to_string())?
        .ok_or("no hub is running (no hub.json)")?;
    if record.app != IDENTITY.app {
        return Err("hub.json is not from this app".to_owned());
    }
    let secret = registry::read_secret(&dir)
        .map_err(|e| e.to_string())?
        .ok_or("no control secret: the hub is not running")?;

    // The secret goes only to the process that says it is this hub.
    let hello = client::request(record.port, "GET", &IDENTITY.route("hello"), &[], "")
        .await
        .map_err(|e| format!("the hub does not answer on port {}: {e}", record.port))?;
    let doc: Value = serde_json::from_str(&hello.body).map_err(|_| "not a hub on that port")?;
    if doc["app"] != IDENTITY.app || doc["instance"].as_str() != Some(record.instance.as_str()) {
        return Err(format!(
            "port {} is not the hub of hub.json (a stale file, or another program)",
            record.port
        ));
    }

    let auth = format!("Bearer {secret}");
    let reply = client::request(
        record.port,
        "POST",
        "/api/control/pair",
        &[
            ("Content-Type", "application/json"),
            ("Authorization", &auth),
        ],
        "{}",
    )
    .await
    .map_err(|e| e.to_string())?;
    if reply.status != 200 {
        return Err(format!(
            "the hub refused the request (HTTP {})",
            reply.status
        ));
    }
    let doc: Value = serde_json::from_str(&reply.body).map_err(|e| e.to_string())?;
    let code = doc["code"].as_str().ok_or("the hub sent no code")?;
    Ok(format!("http://127.0.0.1:{}/#pair={code}", record.port))
}

/// Is this the link `open` prints: the loopback address, a port, `/#pair=` and hex, and nothing
/// else a program could take as an option or a path?
#[must_use]
pub fn is_own_link(url: &str) -> bool {
    let Some(rest) = url.strip_prefix("http://127.0.0.1:") else {
        return false;
    };
    let Some((port, code)) = rest.split_once("/#pair=") else {
        return false;
    };
    !port.is_empty()
        && port.len() <= 5
        && port.bytes().all(|b| b.is_ascii_digit())
        && !code.is_empty()
        && code.len() <= 128
        && code.bytes().all(|b| b.is_ascii_hexdigit())
}

/// Open a link of `open` in the default browser. The link is the hub's own, and nothing else is
/// passed.
///
/// # Errors
///
/// The link is not the hub's own, or the program that opens links could not be started.
pub fn launch(url: &str) -> Result<(), String> {
    if !is_own_link(url) {
        return Err("the link is not the hub's own: not opened".to_owned());
    }
    // The port of the link is the only one the browser helper is allowed to open.
    let port = url
        .strip_prefix("http://127.0.0.1:")
        .and_then(|rest| rest.split_once('/'))
        .and_then(|(port, _)| port.parse::<u16>().ok())
        .ok_or("the link has no port: not opened")?;
    appkit_hub::browser::open(url, &[port])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_hubs_own_link_is_opened() {
        assert!(is_own_link(
            "http://127.0.0.1:17616/#pair=00ff00ff00ff00ff00ff00ff00ff00ff"
        ));
        for bad in [
            "",
            "https://127.0.0.1:17616/#pair=ab",
            "http://localhost:17616/#pair=ab",
            "http://127.0.0.1:17616/#pair=",
            "http://127.0.0.1:17616/#pair=ab cd",
            "http://127.0.0.1:17616/#pair=ab&x=1",
            "http://127.0.0.1:17616/path#pair=ab",
            "http://127.0.0.1:1/../#pair=ab",
            "http://127.0.0.1:17616@evil.example/#pair=ab",
            "--help",
            "file:///C:/Windows/System32/calc.exe",
            "http://127.0.0.1:17616/#pair=zz",
        ] {
            assert!(!is_own_link(bad), "{bad:?}");
        }
    }

    #[test]
    fn a_link_that_is_not_the_hubs_own_is_not_launched() {
        for bad in [
            "file:///C:/Windows/System32/calc.exe",
            "http://127.0.0.1:17616/other",
            "http://evil.example/#pair=ab",
        ] {
            let err = launch(bad).unwrap_err();
            assert!(err.contains("not opened"), "{bad}: {err}");
        }
    }

    #[test]
    fn the_port_comes_from_the_option_then_the_environment_then_the_file() {
        let file = FileConfig::<RulesSettings> {
            port: Some(20000),
            ..FileConfig::default()
        };
        let env = Env::from_pairs([("RULES_DESKTOP_PORT", "30000")]);
        let opts = ServeOptions { port: Some(40000) };
        assert_eq!(wanted_port(&opts, &file, &env).unwrap(), 40000);
        let none = ServeOptions::default();
        assert_eq!(wanted_port(&none, &file, &env).unwrap(), 30000);
        assert_eq!(wanted_port(&none, &file, &Env::default()).unwrap(), 20000);
        assert_eq!(
            wanted_port(&none, &FileConfig::default(), &Env::default()).unwrap(),
            IDENTITY.default_port
        );
        // The variable of the tasks hub is not read.
        let other = Env::from_pairs([("TASKS_DESKTOP_PORT", "30000")]);
        assert_eq!(
            wanted_port(&none, &FileConfig::default(), &other).unwrap(),
            IDENTITY.default_port
        );
    }

    #[test]
    fn a_bad_port_in_the_environment_is_an_error() {
        let none = ServeOptions::default();
        for bad in ["abc", "80", "70000", "-1"] {
            let env = Env::from_pairs([("RULES_DESKTOP_PORT", bad)]);
            assert!(
                wanted_port(&none, &FileConfig::default(), &env).is_err(),
                "{bad:?}"
            );
        }
    }
}
