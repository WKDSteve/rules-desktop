//! The security matrix of appkit (guard, pairing, limits), run for the rules hub, and what the hub
//! answers of its own.

use std::sync::Arc;
use std::time::Duration;

use appkit_core::authn::{ManualClock, Secret, CODE_BYTES, TOKEN_BYTES};
use appkit_core::identity::AppIdentity;
use appkit_hub::{BoxFuture, HubStatus, Limits, RunningServer};
use appkit_testkit::subject::{StartConfig, Subject};
use appkit_testkit::subjects::DEMO_IDENTITY;

mod common;

use common::{get, session};

/// The rules hub, as the matrix starts it.
pub struct Rules;

impl Subject for Rules {
    fn identity(&self) -> &'static AppIdentity {
        &rules_hub::IDENTITY
    }

    fn start(&self, cfg: StartConfig) -> BoxFuture<'_, std::io::Result<RunningServer>> {
        Box::pin(rules_hub::start(rules_hub::Start {
            addr: cfg.addr,
            limits: cfg.limits,
            clock: cfg.clock,
            pairing_ttl: cfg.pairing_ttl,
            control_secret: cfg.control_secret,
            instance: cfg.instance,
            status: cfg.status,
            engine: None,
            host: None,
            sources: Arc::default(),
            watch: None,
            editor: None,
            login: None,
            ui: rules_hub::ui::UiFiles::none(),
            project: None,
            config_dir: None,
        }))
    }

    fn data_routes(&self) -> Vec<&'static str> {
        rules_hub::DATA_ROUTES.to_vec()
    }
}

mod rules {
    appkit_testkit::security_matrix!(crate::Rules);
}

#[tokio::test]
async fn about_names_the_app_to_a_session_and_to_nobody_else() {
    let control = Secret::generate(TOKEN_BYTES).unwrap();
    let server = Rules
        .start(StartConfig {
            addr: "127.0.0.1:0".parse().unwrap(),
            limits: Limits::default(),
            clock: Arc::new(ManualClock::new()),
            pairing_ttl: Duration::from_secs(60),
            control_secret: control.clone(),
            instance: Secret::generate(CODE_BYTES).unwrap(),
            status: HubStatus {
                engine_ready: false,
                problem: None,
                capabilities: Vec::new(),
            },
        })
        .await
        .expect("the hub starts");

    let hello = get(&server, "/api/rules/hello", None).await;
    assert_eq!(hello.status, 200, "{hello:?}");
    assert_eq!(hello.json()["app"], "rules-desktop");

    let about = rules_hub::IDENTITY.route("about");
    assert_eq!(get(&server, &about, None).await.status, 401);

    let token = session(&server, &control).await;

    let r = get(&server, &about, Some(&token)).await;
    assert_eq!(r.status, 200, "{r:?}");
    assert_eq!(r.json()["app"], "rules-desktop");
    assert_eq!(r.json()["version"], rules_hub::version());
    // No engine: the app is still described, and nothing claims an engine that hangs.
    assert!(r.json()["engine"].is_null(), "{r:?}");
    assert!(r.json()["engine_problem"].is_null(), "{r:?}");
}

/// The names of the tasks hub (`tasks_hub::IDENTITY` in tasks-desktop), copied: tasks-hub is not a
/// dependency here, and the two hubs run on one machine side by side.
const TASKS_IDENTITY: AppIdentity = AppIdentity {
    app: "tasks-desktop",
    binary: "tasks-hub",
    env_prefix: "TASKS_DESKTOP",
    title: "Tasks",
    default_port: 17615,
    route_ns: "tasks",
};

/// Every name that lands on the disk, in the registry or in the environment of a person.
fn names(id: &AppIdentity) -> Vec<(&'static str, String)> {
    vec![
        ("app", id.app.to_owned()),
        ("program file", id.program_file()),
        ("config dir variable", id.var("CONFIG_DIR")),
        ("install dir variable", id.var("INSTALL_DIR")),
        ("port variable", id.var("PORT")),
        ("no-login-sync variable", id.var("NO_LOGIN_SYNC")),
        ("desktop file", id.desktop_file()),
        ("agent file", id.agent_file()),
        ("agent label", id.agent_label()),
        ("route of hello", id.route("hello")),
        ("default port", id.default_port.to_string()),
    ]
}

/// The name of the first name two identities share, if any.
fn shared_name(a: &AppIdentity, b: &AppIdentity) -> Option<String> {
    names(a)
        .into_iter()
        .zip(names(b))
        .find(|((_, x), (_, y))| x == y)
        .map(|((what, x), _)| format!("{what}: {x}"))
}

#[test]
fn no_name_of_the_rules_hub_is_a_name_of_another_app() {
    // The app's config folder, install folder and login entry, its variables and its port: a hub
    // that shared one of them with another would write over that one's files or settings.
    for (other, theirs) in [
        ("the demo app", &DEMO_IDENTITY),
        ("tasks-hub", &TASKS_IDENTITY),
    ] {
        assert_eq!(
            shared_name(&rules_hub::IDENTITY, theirs),
            None,
            "a name the same as {other}'s"
        );
    }
}

#[test]
fn a_copy_of_the_names_of_tasks_hub_is_caught() {
    // The likely slip: the identity of tasks-hub taken over with only the port changed.
    let copied = AppIdentity {
        default_port: 17699,
        ..TASKS_IDENTITY
    };
    assert_eq!(
        shared_name(&copied, &TASKS_IDENTITY),
        Some("app: tasks-desktop".to_owned())
    );
    assert_eq!(shared_name(&rules_hub::IDENTITY, &copied), None);
}
