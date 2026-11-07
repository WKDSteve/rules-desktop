//! `rules-hub serve | start | stop | open | autostart | install | uninstall | --version`;
//! without an argument (a double click) it starts the hub in the background and opens the browser.

use rules_hub::app::{self, ServeOptions};
use rules_hub::background::{self, Autostart, InstallOptions, UninstallOptions};

const USAGE: &str = "usage:
  rules-hub                               (no argument, as a double click) start the hub in the
                                          background if it is not running, open the browser
  rules-hub serve [--port <1024-65535>]   run the hub until Ctrl-C
  rules-hub start [--open]                run the hub in the background (no window; log in
                                          the config folder); --open also opens the browser
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
    print!("{}", background::start().await?);
    let url = app::open().await?;
    if std::env::var_os(rules_hub::IDENTITY.var("NO_LAUNCH")).is_some() {
        println!("{url}");
        Ok(())
    } else {
        app::launch(&url)
    }
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
                app::launch(&app::open().await?)
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

#[tokio::main]
async fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let by_click = args.is_empty();
    if let Err(message) = Box::pin(run(args)).await {
        eprintln!("rules-hub: {message}");
        if by_click {
            wait_for_enter();
        }
        std::process::exit(1);
    }
}
