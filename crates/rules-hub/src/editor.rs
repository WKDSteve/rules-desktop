//! Opening a rule file in Neovim, in a window of its own.
//!
//! The page asks by rule id; the hub finds the file (the `source` of the rule in the held
//! snapshot names it, the hub checks that it lies inside the rule sets of the vault) and starts Neovim with that one
//! file as an argument. No shell is involved: a program and its arguments, so
//! nothing in a file name can become a command. Which window Neovim gets
//! depends on the system: a new console on Windows, the first terminal program
//! found on Linux. Where no way is known the answer says so.
//!
//! `RULES_DESKTOP_EDITOR_LOG` (a file) makes the hub write the file it would
//! have opened to that file instead of starting anything: the seam of the tests.

use std::ffi::OsString;
use std::io::Write as _;
use std::path::{Path, PathBuf};

use appkit_hub::env::Env;

/// Something that can show a file in an editor.
pub trait FileOpener: Send + Sync {
    /// Start the editor on `file` (at `line` when given); does not wait for it. The error is a short
    /// text for the page: no path.
    ///
    /// # Errors
    ///
    /// Why the editor could not be started.
    fn open(&self, file: &Path, line: Option<u32>) -> Result<(), String>;
}

/// A program and its arguments.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Launch {
    pub program: PathBuf,
    pub args: Vec<OsString>,
}

/// The terminal programs tried on Linux, in this order, each with what comes
/// before the command it runs.
const TERMINALS: &[(&str, &[&str])] = &[
    ("x-terminal-emulator", &["-e"]),
    ("gnome-terminal", &["--"]),
    ("konsole", &["-e"]),
    ("xfce4-terminal", &["-x"]),
    ("alacritty", &["-e"]),
    ("kitty", &[]),
    ("xterm", &["-e"]),
];

/// How to start `nvim` on `file` in a window of its own, or `None` when no way
/// is known on `os` (`std::env::consts::OS`). `find` looks a program up.
#[must_use]
pub fn plan(
    nvim: &Path,
    file: &Path,
    os: &str,
    find: &dyn Fn(&str) -> Option<PathBuf>,
) -> Option<Launch> {
    match os {
        // A console of its own is made when the process starts (see `spawn`).
        "windows" => Some(Launch {
            program: nvim.to_path_buf(),
            args: vec![file.as_os_str().to_owned()],
        }),
        "linux" => TERMINALS.iter().find_map(|(name, before)| {
            let program = find(name)?;
            let mut args: Vec<OsString> = before.iter().map(OsString::from).collect();
            args.push(nvim.as_os_str().to_owned());
            args.push(file.as_os_str().to_owned());
            Some(Launch { program, args })
        }),
        _ => None,
    }
}

/// A program of this name on `PATH` (as a file).
fn on_path(env: &Env, name: &str) -> Option<PathBuf> {
    let path = env.get("PATH")?;
    std::env::split_paths(path)
        .map(|dir| dir.join(name))
        .find(|candidate| candidate.is_file())
}

/// Starts Neovim in a terminal window.
pub struct TerminalOpener {
    nvim: PathBuf,
    env: Env,
}

impl TerminalOpener {
    #[must_use]
    pub fn new(nvim: PathBuf, env: Env) -> Self {
        Self { nvim, env }
    }
}

#[cfg(windows)]
fn spawn(launch: &Launch) -> std::io::Result<std::process::Child> {
    use std::os::windows::process::CommandExt as _;
    // CREATE_NEW_CONSOLE: Neovim is a console program, and the hub has none.
    std::process::Command::new(&launch.program)
        .args(&launch.args)
        .creation_flags(0x0000_0010)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
}

#[cfg(not(windows))]
fn spawn(launch: &Launch) -> std::io::Result<std::process::Child> {
    std::process::Command::new(&launch.program)
        .args(&launch.args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
}

impl FileOpener for TerminalOpener {
    fn open(&self, file: &Path, line: Option<u32>) -> Result<(), String> {
        if let Some(log) = self.env.path(&crate::IDENTITY.var("EDITOR_LOG")) {
            let mut out = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(log)
                .map_err(|_| "the test log cannot be written".to_owned())?;
            let at = line.map_or_else(String::new, |n| format!(":{n}"));
            writeln!(out, "{}{at}", file.display())
                .map_err(|_| "the test log cannot be written".to_owned())?;
            return Ok(());
        }
        let find = |name: &str| on_path(&self.env, name);
        let mut launch = plan(&self.nvim, file, std::env::consts::OS, &find).ok_or_else(|| {
            "no way to open a window with Neovim is known on this system".to_owned()
        })?;
        // `+N` before the file puts the cursor on the line of the rule.
        if let Some(n) = line {
            let at = launch.args.len().saturating_sub(1);
            launch.args.insert(at, OsString::from(format!("+{n}")));
        }
        let mut child = spawn(&launch).map_err(|_| "Neovim could not be started".to_owned())?;
        // Reaped when it ends; the hub never waits for the editor.
        std::thread::spawn(move || {
            let _ = child.wait();
        });
        Ok(())
    }
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
/// The longest plain path Windows takes without long-path support (`MAX_PATH` is 260 with the terminating NUL).
const PLAIN_PATH_LIMIT: usize = 259;

/// On Windows `canonicalize` answers `\\?\C:\dir\file`; Neovim and most programs want `C:\dir\file`. The prefix stays
/// where the plain form would not do: a network share (`\\?\UNC\...`), a path too long to be plain, and a name Windows
/// reads differently without it (a device name such as `con.md`, a trailing dot or space). Anything else keeps its form.
#[must_use]
pub fn strip_verbatim(path: PathBuf) -> PathBuf {
    let text = path.to_string_lossy();
    match text.strip_prefix("\\\\?\\") {
        Some(rest)
            if rest.as_bytes().get(1) == Some(&b':')
                && rest.encode_utf16().count() <= PLAIN_PATH_LIMIT
                && rest.split('\\').skip(1).all(is_plain_name) =>
        {
            PathBuf::from(rest)
        }
        _ => path,
    }
}

/// A file or folder name that means the same with and without the verbatim prefix.
fn is_plain_name(name: &str) -> bool {
    if name.ends_with('.') || name.ends_with(' ') {
        return false;
    }
    let stem = name
        .split('.')
        .next()
        .unwrap_or_default()
        .trim_end_matches(' ')
        .to_ascii_uppercase();
    let device = matches!(
        stem.as_str(),
        "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$"
    ) || ["COM", "LPT"].iter().any(|prefix| {
        // COM1 to COM9, and the superscript digits Windows reserves too (COM¹, COM², COM³).
        stem.strip_prefix(prefix).is_some_and(|n| {
            matches!(n, "¹" | "²" | "³") || (n.len() == 1 && n.as_bytes()[0].is_ascii_digit())
        })
    });
    !device
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scratch::Scratch;

    fn none(_: &str) -> Option<PathBuf> {
        None
    }

    #[test]
    fn windows_starts_neovim_itself_and_linux_the_first_terminal_it_finds() {
        let nvim = PathBuf::from("/bin/nvim");
        let file = PathBuf::from("/v/rulesets/a/regeln/x.md");
        let win = plan(&nvim, &file, "windows", &none).unwrap();
        assert_eq!(win.program, nvim);
        assert_eq!(win.args, vec![file.as_os_str().to_owned()]);

        let only_konsole =
            |name: &str| (name == "konsole").then(|| PathBuf::from("/usr/bin/konsole"));
        let l = plan(&nvim, &file, "linux", &only_konsole).unwrap();
        assert_eq!(l.program, PathBuf::from("/usr/bin/konsole"));
        assert_eq!(
            l.args,
            vec![
                OsString::from("-e"),
                nvim.as_os_str().to_owned(),
                file.as_os_str().to_owned()
            ]
        );
        // The first of the list wins.
        let both =
            |name: &str| matches!(name, "xterm" | "gnome-terminal").then(|| PathBuf::from(name));
        assert_eq!(
            plan(&nvim, &file, "linux", &both).unwrap().program,
            PathBuf::from("gnome-terminal")
        );
    }

    #[test]
    fn where_no_way_is_known_there_is_none() {
        let nvim = PathBuf::from("nvim");
        let file = PathBuf::from("x.md");
        assert!(plan(&nvim, &file, "linux", &none).is_none());
        assert!(plan(&nvim, &file, "macos", &none).is_none());
    }

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
    fn the_verbatim_prefix_of_a_windows_path_goes_and_a_share_keeps_its_form() {
        assert_eq!(
            strip_verbatim(PathBuf::from("\\\\?\\C:\\v\\a.md")),
            PathBuf::from("C:\\v\\a.md")
        );
        for same in ["\\\\?\\UNC\\server\\share\\a.md", "C:\\v\\a.md", "/v/a.md"] {
            assert_eq!(strip_verbatim(PathBuf::from(same)), PathBuf::from(same));
        }
        // Where the plain form would be read differently or not at all, the prefix stays.
        let long = format!("\\\\?\\C:\\{}\\a.md", "d".repeat(260));
        let edge = format!("\\\\?\\C:\\{}", "e".repeat(259 - 3));
        for kept in [
            long.as_str(),
            "\\\\?\\C:\\v\\con.md",
            "\\\\?\\C:\\v\\NUL",
            "\\\\?\\C:\\v\\com1.txt",
            "\\\\?\\C:\\v\\LPT9.md",
            "\\\\?\\C:\\v\\a.md.",
            "\\\\?\\C:\\v\\a.md ",
            "\\\\?\\C:\\aux\\a.md",
            "\\\\?\\C:\\v\\com¹.md",
            "\\\\?\\C:\\v\\LPT³",
        ] {
            assert_eq!(strip_verbatim(PathBuf::from(kept)), PathBuf::from(kept));
        }
        // 259 characters still go plain, and names that only look like devices do.
        assert_eq!(
            strip_verbatim(PathBuf::from(&edge)),
            PathBuf::from(&edge[4..])
        );
        // The limit counts UTF-16 units, not characters: a name of characters outside the basic plane is twice as long.
        let wide_ok = format!("\\\\?\\C:\\{}", "😀".repeat(128)); // 3 + 256 = 259 units
        let wide_long = format!("\\\\?\\C:\\{}", "😀".repeat(129)); // 3 + 258 = 261 units
        assert_eq!(
            strip_verbatim(PathBuf::from(&wide_ok)),
            PathBuf::from(&wide_ok[4..])
        );
        assert_eq!(
            strip_verbatim(PathBuf::from(&wide_long)),
            PathBuf::from(&wide_long)
        );
        for plain in ["console.md", "com10.md", "auxiliary", "a.con"] {
            let verbatim = format!("\\\\?\\C:\\v\\{plain}");
            assert_eq!(
                strip_verbatim(PathBuf::from(&verbatim)),
                PathBuf::from(format!("C:\\v\\{plain}")),
                "{plain}"
            );
        }
    }

    #[test]
    fn the_test_seam_writes_the_file_it_would_have_opened() {
        let dir = Scratch::new("editor-log");
        let log = dir.join("opened.log");
        let env = Env::from_pairs([(
            "RULES_DESKTOP_EDITOR_LOG",
            log.to_string_lossy().into_owned(),
        )]);
        let opener = TerminalOpener::new(PathBuf::from("nvim"), env);
        opener.open(Path::new("/v/a.md"), None).unwrap();
        opener.open(Path::new("/v/b.md"), Some(7)).unwrap();
        let text = std::fs::read_to_string(&log).unwrap();
        assert_eq!(text.lines().count(), 2);
        assert!(text.contains("a.md") && text.contains("b.md:7"), "{text}");
    }
}
