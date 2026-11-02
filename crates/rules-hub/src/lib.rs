//! The hub of rules-desktop, on `appkit-hub`: its names, the engine seam to rules.nvim
//! (`rules-export/1`, read only), the data classes of this device (`sources.json`) and the routes
//! a session reads (`snapshot`, `rule`, `sources`, `about`).
//! The commands of the program (`serve`, `start`, `stop`, `open`, `autostart`, `install`,
//! `uninstall`) are in `app` and `background`. The page, the live updates and the search arrive
//! with the later steps of the read-only UI phase.

use std::net::SocketAddr;
use std::sync::{Arc, LazyLock};
use std::time::Duration;

use appkit_core::authn::{Clock, Secret};
use appkit_core::identity::AppIdentity;
use appkit_hub::{serve, GuardRules, HubStatus, Limits, RunningServer, ServeParts};

mod answer;
pub mod app;
pub mod background;
pub mod bodies;
pub mod editor;
pub mod engine;
mod indexer;
mod live;
pub mod locate;
pub mod policy;
mod routes;
mod runtime;
pub mod settings;
pub mod sources;
pub mod store;
pub mod ui;
pub mod watch;
pub mod write;

// The one helper that gives a test a folder in the temp dir; the integration tests share the same file.
#[cfg(test)]
#[allow(dead_code)]
#[path = "../tests/common/scratch.rs"]
mod scratch;

pub use routes::{MakeRoute, Shared, MAX_ID_BYTES, ROUTES};
pub use runtime::{Editor, RulesRuntime, Watch};

pub use appkit_hub::{client, loginitems};

use engine::Engine;
use sources::SourcePolicy;

/// The names of the app: its config folder, its program, its variables, its port and its routes.
pub const IDENTITY: AppIdentity = AppIdentity {
    app: "rules-desktop",
    binary: "rules-hub",
    env_prefix: "RULES_DESKTOP",
    title: "Rules",
    default_port: 17616,
    route_ns: "rules",
};

/// What the login entries and the launcher say about this app.
pub const LOGIN: appkit_hub::loginitems::LoginApp<'static> = appkit_hub::loginitems::LoginApp {
    identity: &IDENTITY,
    summary: "the rule set",
};

/// The version of this build, as `rules-hub --version` and `about` say it.
#[must_use]
pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

/// What a build that is not a release says about itself (the commit it was made from): set by the
/// workflow of the rolling build through `RULES_HUB_BUILD`, absent in a release and in a local build.
pub const BUILD: Option<&str> = option_env!("RULES_HUB_BUILD");

/// The line `--version` prints: `rules-hub 0.0.0`, and `rules-hub 0.0.0 (build abc1234)` for a rolling
/// build. A release is exactly the first form (the release workflow compares it with the tag).
#[must_use]
pub fn version_line() -> String {
    version_line_for(BUILD)
}

/// [`version_line`] for a given build (so that both forms are testable: `BUILD` is fixed at compile time).
#[must_use]
pub fn version_line_for(build: Option<&str>) -> String {
    match build {
        Some(build) if !build.is_empty() => {
            format!("{} {} (build {build})", IDENTITY.binary, version())
        }
        _ => format!("{} {}", IDENTITY.binary, version()),
    }
}

/// The routes that need a session, by name under `/api/rules/`: every route of [`ROUTES`], so the
/// security matrix tries each one that is served.
pub static DATA_ROUTES: LazyLock<Vec<&'static str>> =
    LazyLock::new(|| ROUTES.iter().map(|(name, _)| *name).collect());

/// What a start of the hub is given; the rest are the hub's own names.
pub struct Start {
    /// Must be loopback; port 0 picks a free port.
    pub addr: SocketAddr,
    pub limits: Limits,
    pub clock: Arc<dyn Clock>,
    pub pairing_ttl: Duration,
    pub control_secret: Secret,
    pub instance: Secret,
    pub status: HubStatus,
    /// The engine the routes ask; `None`: the hub is up and every data route says the engine is
    /// not ready.
    pub engine: Option<Arc<dyn Engine>>,
    /// Whether the engine's persistent host runs: the texts of the rules are read for the search only while it
    /// does (each is a question of its own). `None`: every question is cheap (an engine in a test), the texts are read.
    pub host: Option<Arc<dyn bodies::HostProbe>>,
    /// The data classes of this device. The caller loads them with [`SourcePolicy::load`] before
    /// it starts the hub and must not start it when that fails (`app::serve` stops the start).
    /// The default classifies nothing.
    pub sources: Arc<SourcePolicy>,
    /// The vault to watch for changes, with the timings of the scan. `None`: nothing is watched,
    /// the snapshot changes when a client asks for a rescan or it is too old.
    pub watch: Option<Watch>,
    /// How a rule file is opened in an editor; `None`: the `open` route says it cannot.
    pub editor: Option<Editor>,
    /// The project whose `.rules-waivers.json` the waivers are kept in (the person's word about this device);
    /// `None`: the waiver routes say there is none. The page never names a folder.
    pub project: Option<std::path::PathBuf>,
    /// The config folder, where `sources.json` is kept and, when the person changes a class on the page, written; `None`:
    /// the hub keeps no such file and says so.
    pub config_dir: Option<std::path::PathBuf>,
    /// The files of the page; [`ui::UiFiles::none`] serves the placeholder page of the core.
    pub ui: ui::UiFiles,
}

/// Starts the hub.
///
/// # Errors
///
/// When the address is not loopback or cannot be bound.
pub async fn start(s: Start) -> std::io::Result<RunningServer> {
    let max_age = s.limits.snapshot_max_age;
    serve(
        ServeParts {
            addr: s.addr,
            limits: s.limits,
            clock: s.clock,
            pairing_ttl: s.pairing_ttl,
            control_secret: s.control_secret,
            instance: s.instance,
            app: IDENTITY.app,
            version: version(),
            build_id: BUILD,
            // `ops` takes writes and dry runs (only its body tells them apart: the writes are counted again,
            // strictly, by the handler) and has the coarse limit of the writes.
            guard_rules: GuardRules::new(IDENTITY.route("hello"), [IDENTITY.route("ops")]),
            ui: Arc::new(s.ui),
            runtime: RulesRuntime::new(
                s.engine, s.host, s.status, s.sources, max_age, s.watch, s.editor,
            )
            .with_project(s.project)
            .with_config_dir(s.config_dir),
            // There is one vault and no list to choose from, so this is never called.
            build: Arc::new(move |()| {
                RulesRuntime::new(
                    None,
                    None,
                    HubStatus {
                        engine_ready: false,
                        problem: None,
                        capabilities: Vec::new(),
                    },
                    Arc::default(),
                    max_age,
                    None,
                    None,
                )
            }),
            vaults: None,
        },
        routes::app_routes,
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_is_semver_shaped() {
        assert_eq!(version().split('.').count(), 3);
    }

    #[test]
    fn a_rolling_build_names_its_commit_and_a_release_does_not() {
        let release = format!("rules-hub {}", version());
        assert_eq!(version_line_for(None), release);
        assert_eq!(version_line_for(Some("")), release);
        assert_eq!(
            version_line_for(Some("abc1234")),
            format!("{release} (build abc1234)")
        );
    }
}
