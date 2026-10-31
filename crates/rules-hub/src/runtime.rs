//! What the hub keeps while it runs: the status `hello` reports, the engine, the snapshot and the
//! data classes of this device.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock, RwLock};
use std::time::Duration;

use appkit_hub::{HubStatus, Runtime};
use axum::body::Bytes;
use serde::Deserialize;
use serde_json::Value;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use crate::bodies::{BodiesEngine, HostProbe};
use crate::editor::FileOpener;
use crate::engine::{Calls, Engine};
use crate::indexer::{self, Indexer};
use crate::locate::Locate;
use crate::sources::{SourceFile, SourcePolicy};
use crate::store::{self, Cached, RefreshOptions, SnapshotStore, StoreOptions, SHAPE};
use crate::watch::{self, Hint, Watcher};
use appkit_hub::search::SearchIndex;

/// What the hub keeps for the vault it reads.
pub struct RulesRuntime {
    pub(crate) status: HubStatus,
    pub(crate) engine: Option<Arc<dyn Engine>>,
    pub(crate) store: Arc<SnapshotStore>,
    /// The data classes of the rule sets on this device (`sources.json`).
    sources: RwLock<Arc<SourcePolicy>>,
    /// Where `sources.json` is kept; none: the classes cannot be changed from the page.
    pub(crate) config_dir: Option<PathBuf>,
    /// One change of the classes at a time.
    pub(crate) policy_edit: Mutex<()>,
    /// What was read of the held snapshot, with the text it was read from.
    held: Mutex<Option<(Bytes, Arc<Held>)>>,
    /// The text of the rules, for the search; with the engine only.
    pub(crate) search: Option<Arc<SearchIndex>>,
    /// The event streams that are open, by session token.
    pub(crate) streams: Arc<StreamCounter>,
    /// Shows a rule file in an editor; none in a hub that cannot.
    pub(crate) editor: Option<Editor>,
    /// The project of the waivers (`config.json`), put into every waiver operation by the hub.
    pub(crate) project: Option<PathBuf>,
    /// The indexer and the refresher; they end with the runtime.
    jobs: Mutex<Vec<JoinHandle<()>>>,
    // Kept alive: the refresher ends when every sender of hints is gone.
    _hints: Option<mpsc::UnboundedSender<Hint>>,
    _watcher: Option<Watcher>,
}

/// How a rule file is shown in an editor, and how its place is found.
#[derive(Clone)]
pub struct Editor {
    /// Says where the file of a rule is: the engine knows, the hub does not work it out from the name.
    pub locator: Arc<dyn Locate>,
    pub opener: Arc<dyn FileOpener>,
}

/// What the hub watches: the vault, and how often it scans.
pub struct Watch {
    pub vault: PathBuf,
    pub refresh: RefreshOptions,
}

/// The event streams that are open, by session token.
#[derive(Default)]
pub(crate) struct StreamCounter {
    by_token: Mutex<HashMap<String, usize>>,
}

/// One open stream; the place is given back when it is dropped.
pub(crate) struct StreamGuard {
    counter: Arc<StreamCounter>,
    token: String,
}

impl StreamCounter {
    pub(crate) fn acquire(self: &Arc<Self>, token: &str, max: usize) -> Option<StreamGuard> {
        let mut map = self
            .by_token
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let open = map.entry(token.to_owned()).or_insert(0);
        if *open >= max {
            return None;
        }
        *open += 1;
        Some(StreamGuard {
            counter: Arc::clone(self),
            token: token.to_owned(),
        })
    }
}

impl Drop for StreamGuard {
    fn drop(&mut self) {
        let mut map = self
            .counter
            .by_token
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(open) = map.get_mut(&self.token) {
            *open = open.saturating_sub(1);
            if *open == 0 {
                map.remove(&self.token);
            }
        }
    }
}

/// What the routes read of a snapshot: the ids of its rules and its files.
pub(crate) struct Held {
    pub(crate) ids: HashSet<String>,
    /// The `source` of each rule that has one (`<rule set name>/<path>`): what the data class of a write is
    /// worked out from.
    pub(crate) places: HashMap<String, String>,
    /// The same for the retired rules (`reinstate` names one of them).
    pub(crate) retired: HashMap<String, String>,
    /// The name of the first rule set (`source.id`): where `new` writes when it names none.
    pub(crate) first_ruleset: Option<String>,
    /// `None` when the snapshot has no `sources`, or one that is not a list of files: the classes
    /// of its files are not known. The ids are read all the same.
    pub(crate) files: Option<Vec<SourceFile>>,
    /// The answer of the `sources` route for these files, computed once for the snapshot.
    pub(crate) sources_body: OnceLock<Bytes>,
}

impl Runtime for RulesRuntime {
    type Target = ();

    fn status(&self) -> &HubStatus {
        &self.status
    }

    fn stop(&self) {
        self.stop_jobs();
    }

    fn setup_required(&self) -> Option<&'static str> {
        setup_reason(self.status.problem)
    }
}

/// The problems of the start that only a person can fix by setting the app up (no vault, or one
/// that is not a folder): `hello` reports them as `setup_required`. A missing Neovim or engine is
/// a problem of the machine, not of the first start.
fn setup_reason(problem: Option<&'static str>) -> Option<&'static str> {
    problem.filter(|p| matches!(*p, "no_vault" | "vault_not_a_directory"))
}

impl Drop for RulesRuntime {
    fn drop(&mut self) {
        self.stop_jobs();
    }
}

/// A snapshot read for the ids of its rules and its files, and nothing else.
#[derive(Deserialize)]
struct SnapshotParts {
    rules: Vec<RuleId>,
    /// Read apart, like the files: what is not right here costs the snapshot its retired rules only.
    #[serde(default)]
    retired: Option<Value>,
    #[serde(default)]
    source: Option<Value>,
    /// Read apart from the rules: a file that is not right must not cost the snapshot its ids.
    #[serde(default)]
    sources: Option<Value>,
}

#[derive(Deserialize)]
struct RuleId {
    id: String,
    #[serde(default)]
    source: Option<String>,
}

impl RulesRuntime {
    /// The folder `sources.json` is kept in.
    #[must_use]
    pub(crate) fn with_config_dir(mut self, dir: Option<PathBuf>) -> Self {
        self.config_dir = dir;
        self
    }

    /// The data classes in force now.
    pub(crate) fn policy(&self) -> Arc<SourcePolicy> {
        Arc::clone(
            &self
                .sources
                .read()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
        )
    }

    /// Put a new policy in force. What was worked out of the held snapshot with the old one is dropped.
    pub(crate) fn set_policy(&self, policy: Arc<SourcePolicy>) {
        *self
            .sources
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = policy;
        *self
            .held
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = None;
    }

    /// The project of the waivers.
    #[must_use]
    pub(crate) fn with_project(mut self, project: Option<PathBuf>) -> Self {
        self.project = project;
        self
    }

    pub(crate) fn new(
        engine: Option<Arc<dyn Engine>>,
        host: Option<Arc<dyn HostProbe>>,
        status: HubStatus,
        sources: Arc<SourcePolicy>,
        max_age: Option<Duration>,
        watch: Option<Watch>,
        editor: Option<Editor>,
    ) -> Self {
        let store = SnapshotStore::new(
            engine.clone(),
            SHAPE,
            StoreOptions {
                max_age,
                ..StoreOptions::default()
            },
        );
        // The text of the rules, for the search: asked of the engine rule by rule, while its host runs.
        let indexing = engine.clone().map(|engine| {
            let bodies = BodiesEngine::new(engine);
            let bodies = Arc::new(match host {
                Some(probe) => bodies.with_host(probe),
                None => bodies,
            });
            let index = SearchIndex::new(Arc::clone(&bodies) as Arc<dyn Engine>, Calls::Bodies);
            (index, bodies)
        });
        let search = indexing.as_ref().map(|(index, _)| Arc::clone(index));
        let mut jobs = Vec::new();
        if let Some((index, bodies)) = indexing {
            jobs.push(indexer::spawn(Indexer::new(index, bodies), &store));
        }
        // The watcher only says "look again"; the refresher does the looking.
        let (mut hints, mut watcher) = (None, None);
        if let Some(watch) = watch.filter(|_| engine.is_some()) {
            let (tx, rx) = mpsc::unbounded_channel();
            match watch::start(&watch.vault, tx.clone()) {
                Ok(started) => watcher = Some(started),
                // Not fatal: the timer and the rescan request still work.
                Err(e) => {
                    eprintln!("note: the vault cannot be watched ({e}); scanning on a timer only");
                }
            }
            jobs.push(store::spawn_refresher(
                Arc::clone(&store),
                rx,
                watch.refresh,
            ));
            hints = Some(tx);
        }
        Self {
            status,
            engine,
            store,
            sources: RwLock::new(sources),
            config_dir: None,
            policy_edit: Mutex::new(()),
            held: Mutex::new(None),
            search,
            streams: Arc::default(),
            editor,
            project: None,
            jobs: Mutex::new(jobs),
            _hints: hints,
            _watcher: watcher,
        }
    }

    /// Stop the refresher and the indexer.
    fn stop_jobs(&self) {
        if let Ok(mut jobs) = self.jobs.lock() {
            for job in jobs.drain(..) {
                job.abort();
            }
        }
    }

    /// What the routes read of a snapshot; `None` when its text is not a snapshot with a list of
    /// rules. Read once for each snapshot the store holds: the text is kept with it, so the same
    /// allocation means the same document.
    pub(crate) fn held(&self, cached: &Cached) -> Option<Arc<Held>> {
        let mut held = self
            .held
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some((text, parts)) = held.as_ref()
            && text.as_ptr() == cached.text.as_ptr()
            && text.len() == cached.text.len()
        {
            return Some(Arc::clone(parts));
        }
        let parts = Arc::new(read_held(&cached.text)?);
        *held = Some((cached.text.clone(), Arc::clone(&parts)));
        Some(parts)
    }

    /// The answer of the `sources` route for a held snapshot; `None` when its files are not known.
    /// The policy changes only when the person changes it (which drops the held snapshot), so it is computed once for
    /// each snapshot.
    pub(crate) fn sources_body(&self, held: &Held) -> Option<Bytes> {
        let files = held.files.as_ref()?;
        Some(
            held.sources_body
                .get_or_init(|| Bytes::from(self.policy().document(files).to_string()))
                .clone(),
        )
    }
}

/// What the routes read of a snapshot's text; `None` without a list of rules.
pub(crate) fn read_held(text: &[u8]) -> Option<Held> {
    let doc: SnapshotParts = serde_json::from_slice(text).ok()?;
    let places = doc
        .rules
        .iter()
        .filter_map(|r| Some((r.id.clone(), r.source.clone()?)))
        .collect();
    let retired = doc
        .retired
        .and_then(|v| serde_json::from_value::<Vec<RuleId>>(v).ok())
        .unwrap_or_default()
        .into_iter()
        .filter_map(|r| Some((r.id, r.source?)))
        .collect();
    let first_ruleset = doc
        .source
        .as_ref()
        .and_then(|v| v.get("id"))
        .and_then(Value::as_str)
        .map(str::to_owned);
    Some(Held {
        places,
        retired,
        first_ruleset,
        ids: doc.rules.into_iter().map(|r| r.id).collect(),
        files: doc
            .sources
            .and_then(|v| serde_json::from_value::<Vec<SourceFile>>(v).ok()),
        sources_body: OnceLock::new(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sources::Class;

    fn runtime() -> RulesRuntime {
        let policy = SourcePolicy::new([("lua".to_owned(), Class::Public)]).unwrap();
        let status = HubStatus {
            engine_ready: false,
            problem: None,
            capabilities: Vec::new(),
        };
        RulesRuntime::new(None, None, status, Arc::new(policy), None, None, None)
    }

    #[test]
    fn only_a_missing_or_unusable_vault_asks_for_the_setup() {
        for (problem, reason) in [
            (Some("no_vault"), Some("no_vault")),
            (Some("vault_not_a_directory"), Some("vault_not_a_directory")),
            (Some("nvim_not_found"), None),
            (Some("engine_not_found"), None),
            (None, None),
        ] {
            assert_eq!(setup_reason(problem), reason, "{problem:?}");
        }
        let mut rt = runtime();
        assert_eq!(rt.setup_required(), None);
        rt.status.problem = Some("no_vault");
        assert_eq!(rt.setup_required(), Some("no_vault"));
    }

    #[test]
    fn a_broken_file_list_costs_the_snapshot_only_its_files() {
        let rt = runtime();
        for sources in [
            r#"[{"path":"lua/DEP.md"}]"#,
            r#"[{"path":"lua/DEP.md","rules":-1}]"#,
            r#"[{"path":"lua/DEP.md","rules":1},7]"#,
            r#""lua""#,
        ] {
            let text = format!(r#"{{"rules":[{{"id":"DEP-01"}}],"sources":{sources}}}"#);
            let held = read_held(text.as_bytes()).expect(sources);
            assert!(held.ids.contains("DEP-01"), "{sources}");
            assert!(held.files.is_none(), "{sources}");
            assert!(rt.sources_body(&held).is_none(), "{sources}");
        }
    }

    #[test]
    fn the_sources_answer_is_computed_once_for_a_snapshot() {
        let rt = runtime();
        let held =
            read_held(br#"{"rules":[],"sources":[{"path":"lua/DEP.md","rules":1}]}"#).unwrap();
        let first = rt.sources_body(&held).unwrap();
        let again = rt.sources_body(&held).unwrap();
        assert_eq!(first.as_ptr(), again.as_ptr(), "computed again");
        let doc: Value = serde_json::from_slice(&first).unwrap();
        assert_eq!(doc["files"][0]["data_class"], "public");
    }
}
