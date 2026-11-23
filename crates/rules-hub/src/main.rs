//! The commands of rules-desktop (the program `rules-hub`; `install` makes it the command `rules-desktop`): those
//! every hub app shares (`appkit_hub::cli`: `start`, `stop`/`quit`, `restart`, `status`, `open`, `link`, `logs`,
//! `paths`, `version`, `help`) and its own (`serve`, `autostart`, `install`, `uninstall`). Without an argument (a
//! double click) it starts the hub in the background and opens the browser.
//!
//! Exit codes: 0 done, 1 failed (`status`: the hub does not run), 2 wrong arguments.
//!
//! The release build is a window program (appkit `docs/PROCESSES.md`, "A hub is a window program"): Windows
//! opens no console window for the link, the login entry or a double click, and from a terminal it attaches to
//! the terminal's console. A debug build stays a console program.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use appkit_core::console::{self, Console};
use appkit_hub::cli::{self, Parsed, Verb};
use rules_hub::app::{self, ServeOptions};
use rules_hub::background::{self, Autostart, InstallOptions, UninstallOptions};

/// The command in the terminal (the program is `rules-hub`).
const COMMAND: &str = "rules-desktop";

/// The commands of rules-desktop that are not common to the hub apps, by the name `help <command>` takes.
const OWN_USAGE: [(&str, &str); 4] = [
    (
        "serve",
        "  rules-desktop serve [--port <1024-65535>]   run the hub in this terminal until Ctrl-C\n",
    ),
    (
        "autostart",
        "  rules-desktop autostart on|off|status       start the hub with your login (Windows: Run key;
                                              Linux: autostart entry; macOS: launch agent)\n",
    ),
    (
        "install",
        "  rules-desktop install [--autostart] [--shortcut]
                                              copy this program to a fixed place (a build then does
                                              not fight the running hub for the file) and make the
                                              command `rules-desktop` for every new terminal; the
                                              Start menu gets a link, --shortcut adds one on the
                                              desktop\n",
    ),
    (
        "uninstall",
        "  rules-desktop uninstall [--purge-settings]  stop the hub, take away the login entry, the link and
                                              the command; --purge-settings also removes the settings
                                              (never the rule sets); the program file is left for the
                                              script\n",
    ),
];

/// The own commands of the app, for the tab completion (`usage` explains `areas`).
fn own_words() -> Vec<&'static str> {
    OWN_USAGE
        .iter()
        .map(|(name, _)| if *name == "usage" { "areas" } else { *name })
        .collect()
}

/// The text of `help`: the common commands, then the own ones, then what the background hub does.
fn usage() -> String {
    let mut out = String::from("usage:\n");
    out.push_str(&cli::usage_lines(COMMAND));
    for (_, lines) in OWN_USAGE {
        out.push_str(lines);
    }
    out.push_str(
        "\nThe hub in the background has no window (its log: rules-desktop logs). It ends itself a while after the
last page closed, unless it starts with your login, a device is paired or keep_running is set in config.json.
A hub that `start --open` or `open` started is stopped again when the page cannot be opened.
Exit codes: 0 done, 1 failed (status: the hub does not run), 2 wrong arguments.",
    );
    out
}

/// What one command does, for `help <command>`: the common text or the own one.
fn help_of(command: &str) -> Option<String> {
    cli::help(command, COMMAND).or_else(|| {
        OWN_USAGE
            .iter()
            .find(|(name, _)| *name == command)
            .map(|(_, lines)| lines.trim_end().to_owned())
    })
}

/// Why the program ends without having done its work, and with which exit code.
struct Failed {
    message: String,
    code: i32,
}

impl Failed {
    /// Wrong arguments: exit code 2.
    fn usage(message: String) -> Self {
        Self {
            message,
            code: cli::USAGE_EXIT,
        }
    }
}

impl From<String> for Failed {
    /// A command that failed: exit code 1.
    fn from(message: String) -> Self {
        Self { message, code: 1 }
    }
}

fn parse_port(args: &[String]) -> Result<Option<u16>, Failed> {
    match args {
        [] => Ok(None),
        [flag, value] if flag == "--port" => value
            .parse::<u16>()
            .ok()
            .filter(|p| *p >= 1024)
            .map(Some)
            .ok_or_else(|| {
                Failed::usage(format!(
                    "--port must be a number from 1024 to 65535, not {value:?}"
                ))
            }),
        _ => Err(Failed::usage(usage())),
    }
}

/// Open the page of the hub that was just started or found, in the browser (`print_only`: print the link instead).
/// Its future is on the heap and made here, not in the `match` of `run`: a large future there takes room in the stack
/// of the main thread for every command, which overflows it in a debug build.
fn open_page(
    started: String,
    print_only: bool,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), String>>>> {
    Box::pin(async move {
        let page = async {
            let url = app::open().await?;
            if print_only {
                println!("{url}");
                Ok(())
            } else {
                app::launch(&url)
            }
        };
        open_or_stop(&started, page, background::stop).await
    })
}

/// Open the page of the hub that was just started or found (`started` is what the start printed). A hub this call
/// started for the page does not stay behind when the page cannot be opened: nobody would see it, and in the
/// background it has no window to close. One that ran already is not this call's to stop.
async fn open_or_stop<P, S, F>(started: &str, page: P, stop: S) -> Result<(), String>
where
    P: std::future::Future<Output = Result<(), String>>,
    S: FnOnce() -> F,
    F: std::future::Future<Output = Result<String, String>>,
{
    let Err(why) = page.await else {
        return Ok(());
    };
    if !appkit_hub::background::started_now(started) {
        return Err(why);
    }
    Err(match stop().await {
        Ok(_) => format!("{why}; the hub that was started for the page is stopped again"),
        Err(left) => format!("{why}; the hub that was started for the page still runs: {left}"),
    })
}

/// A console that was opened by a double click closes with the program: an error is read before that.
fn wait_for_enter() {
    use std::io::{BufRead, IsTerminal, Write};
    if std::io::stdin().is_terminal() {
        eprint!("Press Enter to close this window.");
        let _ = std::io::stderr().flush();
        let _ = std::io::stdin().lock().lines().next();
    }
}

/// Whether the link of a page is printed instead of opened (`RULES_DESKTOP_NO_LAUNCH`: the tests).
fn print_only() -> bool {
    std::env::var_os(rules_hub::IDENTITY.var("NO_LAUNCH")).is_some()
}

/// The hub in the background, then its page in the browser (`start --open`, `open`, `restart --open`).
async fn start_and_open() -> Result<(), String> {
    let started = background::start().await?;
    print!("{started}");
    open_page(started, print_only()).await
}

/// The command the arguments name; the exit code when it ran. Apart from `main` so that its future lives on the heap:
/// the main thread of Windows has 1 MB of stack, and the future of all the commands together is too big for it in a
/// debug build; each command is boxed on its own.
async fn run(args: Vec<String>) -> Result<i32, Failed> {
    let verb = match cli::parse(&args) {
        Parsed::Common(verb) => verb,
        Parsed::Usage(text) => return Err(Failed::usage(text)),
        Parsed::Own => return Box::pin(run_own(&args)).await.map(|()| 0),
    };
    match verb {
        Verb::Click | Verb::Start { open: true } | Verb::Open => {
            Box::pin(start_and_open()).await?;
        }
        Verb::Start { open: false } => print!("{}", Box::pin(background::start()).await?),
        Verb::Stop => print!("{}", Box::pin(background::stop()).await?),
        Verb::Restart { open } => {
            print!("{}", Box::pin(background::stop()).await?);
            if open {
                Box::pin(start_and_open()).await?;
            } else {
                print!("{}", Box::pin(background::start()).await?);
            }
        }
        Verb::Status { json } => {
            let status = Box::pin(cli::status(&background::APP, rules_hub::version())).await;
            if json {
                println!("{}", status.json());
            } else {
                print!("{}", status.text());
            }
            return Ok(status.exit_code());
        }
        Verb::Link => println!("{}", Box::pin(app::open()).await?),
        Verb::Logs { lines, follow } => {
            Box::pin(cli::logs(&background::APP, lines, follow)).await?;
        }
        Verb::Paths => print!("{}", cli::paths(&background::APP)),
        Verb::Completions(shell) => print!(
            "{}",
            cli::completions_script(shell, &[COMMAND, "rules-hub"], &own_words())
        ),
        Verb::Version => println!("{}", rules_hub::version_line()),
        Verb::Help(None) => println!("{}", usage()),
        Verb::Help(Some(command)) => match help_of(&command) {
            Some(text) => println!("{text}"),
            None => {
                return Err(Failed::usage(format!(
                    "no command {command:?}\n\n{}",
                    usage()
                )));
            }
        },
    }
    Ok(0)
}

/// The commands of rules-desktop itself.
async fn run_own(args: &[String]) -> Result<(), Failed> {
    match args.first().map(String::as_str) {
        Some("serve") => {
            let port = parse_port(&args[1..])?;
            Box::pin(app::serve(ServeOptions { port })).await?;
        }
        Some("install") => {
            let mut opts = InstallOptions::default();
            for arg in &args[1..] {
                match arg.as_str() {
                    "--autostart" => opts.autostart = true,
                    "--shortcut" => opts.shortcut = true,
                    _ => return Err(Failed::usage(usage())),
                }
            }
            print!("{}", Box::pin(background::install(opts)).await?);
        }
        Some("uninstall")
            if args.len() == 1 || (args.len() == 2 && args[1] == "--purge-settings") =>
        {
            let opts = UninstallOptions {
                purge_settings: args.len() == 2,
            };
            print!("{}", Box::pin(background::uninstall(opts)).await?);
        }
        Some("autostart") if args.len() == 2 => {
            let what = match args[1].as_str() {
                "on" => Autostart::On,
                "off" => Autostart::Off,
                "status" => Autostart::Status,
                _ => return Err(Failed::usage(usage())),
            };
            print!("{}", background::autostart(what)?);
        }
        _ => return Err(Failed::usage(usage())),
    }
    Ok(())
}

fn main() {
    // First, before the runtime: on Linux and macOS this program is also the watcher of the process group of each
    // child it starts (started with `--appkit-child-watcher`, it only waits and never returns here); docs/PROCESSES.md
    // of appkit.
    appkit_core::child::watcher_main();
    hub_main();
}

#[tokio::main]
async fn hub_main() {
    // Before the first line is written.
    let console = console::attach_parent();
    console::report_panics("rules-hub");
    let args: Vec<String> = std::env::args().skip(1).collect();
    let by_click = args.is_empty();
    match Box::pin(run(args)).await {
        Ok(0) => {}
        // `status` of a hub that does not run: said on stdout already, only the exit code is left.
        Ok(code) => std::process::exit(code),
        Err(failed) => {
            // stderr, or a message box when nobody reads it (the link of a window program).
            console::report("rules-hub", &failed.message);
            // In the console of a terminal the shell reads the same keys: only a console of its own waits.
            if by_click && console == Console::Own {
                wait_for_enter();
            }
            std::process::exit(failed.code);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::open_or_stop;
    use std::cell::Cell;

    const STARTED: &str = "the hub runs in the background (port 1, process 2)\n";
    const FOUND: &str = "a hub is already running (port 1, process 2)\n";

    async fn run(
        started: &str,
        page: Result<(), String>,
        stop: Result<String, String>,
    ) -> (Result<(), String>, bool) {
        let stopped = Cell::new(false);
        let said = open_or_stop(started, async { page }, || {
            stopped.set(true);
            async { stop }
        })
        .await;
        (said, stopped.get())
    }

    #[tokio::test]
    async fn a_hub_started_for_a_page_that_does_not_open_is_stopped_again() {
        let (said, stopped) = run(STARTED, Err("no browser".to_owned()), Ok(String::new())).await;
        assert!(stopped);
        let why = said.unwrap_err();
        assert!(
            why.starts_with("no browser; ") && why.ends_with("is stopped again"),
            "{why}"
        );

        let (said, stopped) = run(
            STARTED,
            Err("no browser".to_owned()),
            Err("denied".to_owned()),
        )
        .await;
        assert!(stopped);
        assert!(said.unwrap_err().ends_with("still runs: denied"));
    }

    #[tokio::test]
    async fn a_hub_that_ran_already_or_a_page_that_opened_stops_nothing() {
        let (said, stopped) = run(FOUND, Err("no browser".to_owned()), Ok(String::new())).await;
        assert_eq!(said, Err("no browser".to_owned()));
        assert!(!stopped, "the hub was not started by this call");

        let (said, stopped) = run(STARTED, Ok(()), Ok(String::new())).await;
        assert_eq!(said, Ok(()));
        assert!(!stopped);
    }
}
