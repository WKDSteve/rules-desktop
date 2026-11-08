//! Opening a rule file in Neovim, in a window of its own.
//!
//! The page asks by rule id; the hub finds the file (the engine locates it, the hub checks here that
//! it lies inside the rule sets of the vault) and the opener of appkit starts Neovim with that one
//! file as an argument, at the line of the rule (`appkit_hub::editor`).
//!
//! `RULES_DESKTOP_EDITOR_LOG` (a file) makes the hub write the file it would have opened to that
//! file instead of starting anything: the seam of the tests.

use std::path::{Path, PathBuf};

pub use appkit_hub::editor::{plan, strip_verbatim, FileOpener, Launch, TerminalOpener};

/// The opener of this app: `nvim` in a window of its own, the seam named by [`crate::IDENTITY`].
#[must_use]
pub fn terminal_opener(nvim: PathBuf, env: appkit_hub::env::Env) -> TerminalOpener {
    TerminalOpener::new(nvim, env, &crate::IDENTITY)
}

/// The file of a rule as the engine located it, checked against the ruleset the engine says it was loaded from:
/// it must be a plain file (no link) that lies inside `root` once links are resolved (a ruleset that is a file is that
/// file). A link planted in a ruleset does not lead out of it, and a file the engine did not load from `root` is not
/// opened. What the file is called in a document (`source`) plays no part: the place comes from the engine.
///
/// # Errors
///
/// A short reason, no path.
pub fn file_in_ruleset(root: &Path, file: &Path) -> Result<PathBuf, &'static str> {
    if !file.is_absolute() || !root.is_absolute() {
        return Err("the engine named a place that is not a whole path");
    }
    let meta = std::fs::symlink_metadata(file).map_err(|_| "the file is not there")?;
    if meta.file_type().is_symlink() || !meta.is_file() {
        return Err("the rule file is not a plain file");
    }
    let real = file.canonicalize().map_err(|_| "the file is not there")?;
    let real_root = root
        .canonicalize()
        .map_err(|_| "the rule set is not there")?;
    if real != real_root && !real.starts_with(&real_root) {
        return Err("the file lies outside its rule set");
    }
    Ok(strip_verbatim(real))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scratch::Scratch;

    fn scratch(name: &str) -> Scratch {
        let dir = Scratch::new(&format!("editor-{name}"));
        std::fs::create_dir_all(dir.join("set").join("a")).unwrap();
        std::fs::create_dir_all(dir.join("other")).unwrap();
        std::fs::write(dir.join("set").join("a").join("x.md"), "x").unwrap();
        std::fs::write(dir.join("other").join("y.md"), "y").unwrap();
        dir
    }

    #[test]
    fn a_file_of_the_ruleset_is_accepted_and_one_beside_it_is_not() {
        let dir = scratch("in");
        let root = dir.join("set");
        let ok = file_in_ruleset(&root, &root.join("a").join("x.md")).unwrap();
        // What Neovim gets is a plain path, not the verbatim form Windows' canonicalize gives.
        assert!(!ok.to_string_lossy().starts_with(r"\\?\"), "{ok:?}");
        // The engine names a file of another ruleset, one that is not there, a folder, a climb, a relative place.
        for bad in [
            dir.join("other").join("y.md"),
            root.join("a").join("nope.md"),
            root.join("a"),
            root.join("a")
                .join("..")
                .join("..")
                .join("other")
                .join("y.md"),
            PathBuf::from("a/x.md"),
        ] {
            assert!(
                file_in_ruleset(&root, &bad).is_err(),
                "{bad:?} was accepted"
            );
        }
        assert!(file_in_ruleset(Path::new("set"), &root.join("a").join("x.md")).is_err());
    }

    #[test]
    fn a_ruleset_that_is_a_file_is_that_file() {
        let dir = scratch("file");
        let file = dir.join("other").join("y.md");
        assert!(file_in_ruleset(&file, &file).is_ok());
        assert!(file_in_ruleset(&file, &dir.join("set").join("a").join("x.md")).is_err());
    }

    #[test]
    fn the_test_seam_is_named_by_the_app_and_has_the_line() {
        let dir = Scratch::new("editor-log");
        let log = dir.join("opened.log");
        let env = appkit_hub::env::Env::from_pairs([(
            "RULES_DESKTOP_EDITOR_LOG",
            log.to_string_lossy().into_owned(),
        )]);
        terminal_opener(PathBuf::from("nvim"), env)
            .open(Path::new("/v/b.md"), Some(7))
            .unwrap();
        assert!(std::fs::read_to_string(&log).unwrap().contains("b.md:7"));
    }
}
