//! Where the file of a rule is. The `source` of a rule is a name (`<ruleset name>/<path>`, with the folder
//! above on a clash and `#n` after that) and not a place; the one that knows the place is the engine, and the
//! naming rule is not rebuilt here. rules.nvim answers `scripts/rules.lua locate <id>` with two plain lines, the
//! file and the ruleset it was loaded from. That is not a document of `rules-export/1` (which holds no path of
//! this machine), so it is a question of its own, asked of a process of its own, with the environment the engine
//! gets and nothing else.

use std::path::PathBuf;
use std::time::Duration;

use appkit_core::child::{self, Environment, Leftovers, Run, Start};
use appkit_core::proc::{Output, ProcError};
use appkit_hub::engine::{child_env, CliEngineConfig, EngineVars};
use appkit_hub::BoxFuture;

/// Where a rule's file is and which ruleset holds it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Located {
    pub file: PathBuf,
    /// `None`: the engine knows no ruleset that holds the file.
    pub root: Option<PathBuf>,
}

/// Asks where the file of a rule is.
pub trait Locate: Send + Sync {
    /// # Errors
    ///
    /// A short word for the page: no path, no text of the engine.
    fn locate<'a>(&'a self, id: &'a str) -> BoxFuture<'a, Result<Located, &'static str>>;
}

/// The engine as a process: one `rules.lua locate` for each question.
pub struct ProcessLocator {
    cfg: CliEngineConfig,
}

/// The longest the question may take.
const TIMEOUT: Duration = Duration::from_secs(30);

/// The longest answer and the longest complaint that are read: two paths fit many times over, and an engine that
/// writes more is ended instead of filling the memory of the hub.
const MAX_ANSWER: usize = 64 * 1024;

/// Who starts the process, as the log of the hub names it.
const CALLER: &str = "rules-hub locate";

impl ProcessLocator {
    #[must_use]
    pub fn new(cfg: CliEngineConfig) -> Self {
        Self { cfg }
    }

    /// The call of the engine: through the helper of appkit (rule RS-08), so it has no window, ends with the hub
    /// and ends with everything it started when it takes too long or writes too much. The path of Neovim is the
    /// one of the settings, taken as it is.
    fn call(&self, id: &str) -> Run {
        let vars = EngineVars {
            dialect: self.cfg.dialect,
            vault: &self.cfg.vault,
            lib_dir: self.cfg.lib_dir.as_deref(),
            extra_areas: &[],
        };
        Run {
            start: Start::as_given(CALLER, &self.cfg.nvim)
                .args(locate_args(&self.cfg.script, id))
                .env(Environment::Only(child_env(&self.cfg.env, Some(&vars)))),
            stdin: Vec::new(),
            timeout: TIMEOUT,
            max_stdout: MAX_ANSWER,
            max_stderr: MAX_ANSWER,
            leftovers: Leftovers::End,
        }
    }

    async fn ask(&self, id: &str) -> Result<Located, &'static str> {
        answer(child::run(self.call(id)).await)
    }
}

/// What the page is told for the end of the call.
fn answer(ran: Result<Output, ProcError>) -> Result<Located, &'static str> {
    let output = match ran {
        Ok(output) => output,
        Err(ProcError::Spawn(_)) => return Err("the engine could not be started"),
        Err(ProcError::Timeout(_)) => return Err("the engine did not answer in time"),
        Err(ProcError::TooLarge { .. }) => return Err("the engine answered too much"),
        Err(ProcError::Io(_)) => return Err("the engine could not be read"),
    };
    match output.code {
        Some(0) => parse(&String::from_utf8_lossy(&output.stdout)),
        // The engine does not hold the id: the page was told by the snapshot it does, so the vault changed.
        Some(1) => Err("the engine does not hold this rule"),
        _ => Err("the engine could not tell where the file is"),
    }
}

/// The arguments of the question. The id comes after `--`: a rule id may look like a flag (`--trust=C:/x` is a
/// loadable id), and the engine would read it as one and refuse the question.
fn locate_args(script: &std::path::Path, id: &str) -> Vec<std::ffi::OsString> {
    let mut args: Vec<std::ffi::OsString> = ["--headless", "-u", "NONE", "-i", "NONE", "-n", "-l"]
        .into_iter()
        .map(Into::into)
        .collect();
    args.push(script.into());
    args.extend(["locate", "--", id].map(Into::into));
    args
}

/// The two lines of the answer: the file, then the ruleset (empty: none).
fn parse(text: &str) -> Result<Located, &'static str> {
    let mut lines = text.lines();
    let file = lines
        .next()
        .map(str::trim_end)
        .filter(|l| !l.is_empty())
        .ok_or("the engine named no file")?;
    let root = lines.next().map(str::trim_end).filter(|l| !l.is_empty());
    if lines.next().is_some() {
        return Err("the engine answered more than two lines");
    }
    Ok(Located {
        file: PathBuf::from(file),
        root: root.map(PathBuf::from),
    })
}

impl Locate for ProcessLocator {
    fn locate<'a>(&'a self, id: &'a str) -> BoxFuture<'a, Result<Located, &'static str>> {
        Box::pin(self.ask(id))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scratch::Scratch;

    #[test]
    fn two_lines_are_the_file_and_the_ruleset() {
        let got = parse("/v/a/b.md\n/v/a\n").unwrap();
        assert_eq!(got.file, PathBuf::from("/v/a/b.md"));
        assert_eq!(got.root, Some(PathBuf::from("/v/a")));
        // A Windows engine may end its lines with CR LF.
        let got = parse("C:/v/a/b.md\r\nC:/v/a\r\n").unwrap();
        assert_eq!(got.file, PathBuf::from("C:/v/a/b.md"));
        assert_eq!(got.root, Some(PathBuf::from("C:/v/a")));
        // No ruleset named: the file is there, and nothing says where it was loaded from.
        assert_eq!(parse("/v/a/b.md\n\n").unwrap().root, None);
        assert_eq!(parse("/v/a/b.md\n").unwrap().root, None);
    }

    #[test]
    fn the_id_comes_after_the_end_of_the_flags() {
        let args = locate_args(std::path::Path::new("rules.lua"), "--trust=C:/x");
        let tail: Vec<_> = args.iter().rev().take(3).collect();
        assert_eq!(tail, ["--trust=C:/x", "--", "locate"]);
    }

    #[test]
    fn anything_else_is_not_an_answer() {
        for bad in ["", "\n", "\n/v\n", "/a\n/b\n/c\n"] {
            assert!(parse(bad).is_err(), "{bad:?}");
        }
    }

    /// Neovim of this machine, as the tests of the engine use it.
    fn nvim() -> PathBuf {
        let name = if cfg!(windows) { "nvim.exe" } else { "nvim" };
        std::env::split_paths(&std::env::var_os("PATH").expect("PATH is set"))
            .map(|dir| dir.join(name))
            .find(|candidate| candidate.is_file())
            .expect("the test of locate needs nvim on the PATH")
    }

    /// A locator whose "engine" is the Lua script `body`.
    fn locator(folder: &Scratch, body: &str) -> ProcessLocator {
        let script = folder.join("engine.lua");
        std::fs::write(&script, body).unwrap();
        ProcessLocator::new(CliEngineConfig {
            dialect: crate::engine::DIALECT,
            nvim: nvim(),
            script,
            vault: folder.to_path_buf(),
            lib_dir: None,
            extra_areas: Vec::new(),
            env: appkit_hub::env::Env::from_process(),
            timeout: TIMEOUT,
            max_queue: 1,
            max_reply_bytes: MAX_ANSWER,
        })
    }

    #[tokio::test]
    async fn a_real_process_answers_through_the_helper() {
        let folder = Scratch::new("locate-answer");
        let found = locator(&folder, "io.write('/v/a/b.md\\n/v/a\\n')")
            .ask("A-01")
            .await
            .unwrap();
        assert_eq!(found.file, PathBuf::from("/v/a/b.md"));
        assert_eq!(found.root, Some(PathBuf::from("/v/a")));
        let missing = locator(&folder, "os.exit(1)").ask("A-01").await;
        assert_eq!(missing, Err("the engine does not hold this rule"));
    }

    /// An engine that writes without end is ended and its text is not taken for a path: before the helper the
    /// whole output was read into memory and its first line was the "file".
    #[tokio::test]
    async fn an_answer_past_the_limit_ends_the_engine() {
        let folder = Scratch::new("locate-flood");
        let flood = format!("io.write(string.rep('x', {}))", MAX_ANSWER * 4);
        let got = locator(&folder, &flood).ask("A-01").await;
        assert_eq!(got, Err("the engine answered too much"));
    }

    #[test]
    fn the_call_is_bounded_and_leaves_nothing_behind() {
        let folder = Scratch::new("locate-call");
        let run = locator(&folder, "").call("A-01");
        assert_eq!(run.timeout, TIMEOUT);
        assert_eq!((run.max_stdout, run.max_stderr), (MAX_ANSWER, MAX_ANSWER));
        assert_eq!(run.leftovers, Leftovers::End);
        assert_eq!(run.start.caller(), CALLER);
    }
}
