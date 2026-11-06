//! stdout/stderr writing and terminal colors.
//!
//! Every user-facing line goes through [`emit`]. Rust ignores SIGPIPE, so a
//! closed reader (`wt list | head -1`) surfaces as an `EPIPE` write error; that
//! ends the process quietly instead of a panic. Read-only commands then exit 0
//! (see [`read_only`]); everything else exits 1, because it stopped before
//! finishing what it was asked to change.

use std::io::{self, IsTerminal, Write};
use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::OnceLock;

static COLORS: OnceLock<bool> = OnceLock::new();
static EPIPE_EXIT: AtomicI32 = AtomicI32::new(1);

/// Mark the running command as read-only: a closed stdout then ends it with
/// exit 0, the way `wt list | head -1` should.
pub fn read_only() {
    EPIPE_EXIT.store(0, Ordering::Relaxed);
}

/// Decide once whether ANSI colors are used: off when `NO_COLOR` is set (even
/// to an empty string) or stdout is not a terminal.
pub fn init_colors() {
    let enabled = std::env::var_os("NO_COLOR").is_none() && io::stdout().is_terminal();
    let _ = COLORS.set(enabled);
}

fn enabled() -> bool {
    *COLORS.get().unwrap_or(&false)
}

macro_rules! color {
    ($name:ident, $code:expr) => {
        pub fn $name() -> &'static str {
            if enabled() {
                $code
            } else {
                ""
            }
        }
    };
}

color!(red, "\x1b[0;31m");
color!(green, "\x1b[0;32m");
color!(yellow, "\x1b[0;33m");
color!(blue, "\x1b[0;34m");
color!(bold, "\x1b[1m");
color!(reset, "\x1b[0m");

/// `text` wrapped in `color` and a reset.
pub fn colorize(text: &str, color: &str) -> String {
    format!("{color}{text}{}", reset())
}

fn handle(result: io::Result<()>) {
    if let Err(err) = result {
        if err.kind() == io::ErrorKind::BrokenPipe {
            std::process::exit(EPIPE_EXIT.load(Ordering::Relaxed));
        }
        // Nothing sensible can be printed when stdout itself is broken.
        std::process::exit(1);
    }
}

/// Write `text` plus a newline to stdout.
pub fn emit(text: &str) {
    let mut out = io::stdout().lock();
    handle(writeln!(out, "{text}"));
}

/// Write `text` to stdout without a newline and flush (prompts).
pub fn emit_flush(text: &str) {
    let mut out = io::stdout().lock();
    handle(write!(out, "{text}").and_then(|()| out.flush()));
}

/// Write `text` plus a newline to stderr.
pub fn emit_err(text: &str) {
    let mut err = io::stderr().lock();
    let _ = writeln!(err, "{text}");
}

/// Print an unexpected failure and exit 1, the way an uncaught Python
/// exception ended the old CLI (minus the traceback).
pub fn fatal(msg: &str) -> ! {
    let _ = io::stdout().flush();
    emit_err(&format!("Error: {msg}"));
    std::process::exit(1);
}

/// `eprintln!`-style shorthand for [`emit_err`]. Errors carry no colors:
/// whether stdout is a terminal says nothing about stderr.
#[macro_export]
macro_rules! say_err {
    () => {
        $crate::out::emit_err("")
    };
    ($($arg:tt)*) => {
        $crate::out::emit_err(&format!($($arg)*))
    };
}

/// `println!`-style shorthand for [`emit`].
#[macro_export]
macro_rules! say {
    () => {
        $crate::out::emit("")
    };
    ($($arg:tt)*) => {
        $crate::out::emit(&format!($($arg)*))
    };
}
