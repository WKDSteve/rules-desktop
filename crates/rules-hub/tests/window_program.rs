//! The release build is a window program: Windows opens no console window for the link, the login entry or a
//! double click (appkit `docs/PROCESSES.md`, "A hub is a window program"). A debug build stays a console
//! program on purpose, so the attribute is proven only by `cargo test --release` (a step of the CI).
#![cfg(windows)]

use appkit_testkit::window_program::{subsystem, CONSOLE, WINDOW};

#[test]
fn the_release_build_is_a_window_program_and_a_debug_build_a_console_program() {
    let exe = std::fs::read(env!("CARGO_BIN_EXE_rules-hub")).unwrap();
    let want = if cfg!(debug_assertions) {
        CONSOLE
    } else {
        WINDOW
    };
    assert_eq!(subsystem(&exe), Some(want));
}
