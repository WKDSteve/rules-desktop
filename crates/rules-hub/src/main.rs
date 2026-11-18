//! `rules-hub serve | start | stop | open | autostart | install | uninstall | --version`;
//! without an argument (a double click) it starts the hub in the background and opens the browser.
//!
//! The release build is a window program (appkit `docs/PROCESSES.md`, "A hub is a window program"): Windows
//! opens no console window for the link, the login entry or a double click, and from a terminal it attaches to
//! the terminal's console. A debug build stays a console program.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use appkit_core::console::{self, Console};
use rules_hub::app::{self, ServeOptions};
use rules_hub::background::{self, Autostart, InstallOptions, UninstallOptions};

const USAGE: &str = "usage:
  rules-hub                               (no argument, as a double click) start the hub in the
                                          background if it is not running, open the browser
  rules-hub serve [--port <1024-65535>]   run the hub until Ctrl-C
  rules-hub start [--open]                run the hub in the background (no window; log in
                                          the config folder); --open also opens the browser.
                                          It ends itself a while after the last page closed,
                                          unless it starts with your login, a device is paired
                                          or keep_running is set in config.json
  rules-hub stop                          stop the hub that runs in the background
  rules-hub autostart on|off|status       start the hub with your login (Windows: Run key;
                                          Linux: autostart entry; macOS: launch agent)
  rules-hub install [--autostart] [--shortcut]
                                          copy this program to a fixed place (a build
                                          then does not fight the running hub for the file)
                                          The Start menu gets a link; --shortcut adds one on
                                          the desktop
  rules-hub uninstall [--purge-settings]  stop the hub, take away the login entry and the link;
                                          --purge-settings also removes the settings (never the
                                          rule sets); the program file is left for the script
  rules-hub open [--launch]               print a one-time pairing link for the running hub
                                          (--launch also opens it in the default browser)
  rules-hub --version";

fn parse_port(args: &[String]) -> Result<Option<u16>, String> {
    match args {
        [] => Ok(None),
        [flag, value] if flag == "--port" => value
            .parse::<u16>()
            .ok()
            .filter(|p| *p >= 1024)
            .map(Some)
            .ok_or_else(|| format!("--port must be a number from 1024 to 65535, not {value:?}")),
        _ => Err(USAGE.to_owned()),
    }
}

/// What a double click does: the hub runs in the background (a second click finds it), and the
/// browser opens with a one-time link. `RULES_DESKTOP_NO_LAUNCH` prints the link instead (the tests).
async fn click() -> Result<(), String> {
    let started = background::start().await?;
    print!("{started}");
    let print_only = std::env::var_os(rules_hub::IDENTITY.var("NO_LAUNCH")).is_some();
    open_page(started, print_only).await
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

/// The verb the arguments name. Apart from `main` so that its future lives on the heap: the main
/// thread of Windows has 1 MB of stack.
async fn run(args: Vec<String>) -> Result<(), String> {
    match args.first().map(String::as_str) {
        None => click().await,
        Some("help" | "--help" | "-h") => {
            println!("{USAGE}");
            Ok(())
        }
        Some("serve") => match parse_port(&args[1..]) {
            Ok(port) => app::serve(ServeOptions { port }).await,
            Err(e) => Err(e),
        },
        Some("start") if args.len() == 1 || (args.len() == 2 && args[1] == "--open") => {
            let text = background::start().await?;
            print!("{text}");
            if args.len() == 2 {
                open_page(text, false).await
            } else {
                Ok(())
            }
        }
        Some("install") => {
            let mut opts = InstallOptions::default();
            let mut bad = false;
            for arg in &args[1..] {
                match arg.as_str() {
                    "--autostart" => opts.autostart = true,
                    "--shortcut" => opts.shortcut = true,
                    _ => bad = true,
                }
            }
            if bad {
                Err(USAGE.to_owned())
            } else {
                background::install(opts).await.map(|t| print!("{t}"))
            }
        }
        Some("uninstall")
            if args.len() == 1 || (args.len() == 2 && args[1] == "--purge-settings") =>
        {
            let opts = UninstallOptions {
                purge_settings: args.len() == 2,
            };
            background::uninstall(opts).await.map(|t| print!("{t}"))
        }
        Some("stop") if args.len() == 1 => background::stop().await.map(|t| print!("{t}")),
        Some("autostart") if args.len() == 2 => match args[1].as_str() {
            "on" => Ok(Autostart::On),
            "off" => Ok(Autostart::Off),
            "status" => Ok(Autostart::Status),
            _ => Err(USAGE.to_owned()),
        }
        .and_then(background::autostart)
        .map(|t| print!("{t}")),
        Some("open") if args.len() == 1 => app::open().await.map(|url| println!("{url}")),
        Some("open") if args.len() == 2 && args[1] == "--launch" => {
            let url = app::open().await?;
            println!("{url}");
            app::launch(&url)
        }
        Some("--version" | "-V") => {
            println!("{}", rules_hub::version_line());
            Ok(())
        }
        _ => Err(USAGE.to_owned()),
    }
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
    if let Err(message) = Box::pin(run(args)).await {
        // stderr, or a message box when nobody reads it (the link of a window program).
        console::report("rules-hub", &message);
        // In the console of a terminal the shell reads the same keys: only a console of its own waits.
        if by_click && console == Console::Own {
            wait_for_enter();
        }
        std::process::exit(1);
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
