//! clv: a file's pages shown in the terminal through the kitty graphics protocol.
//!
//! Usage: `clv --help`, which prints [`view::usage`].

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

#[cfg(unix)]
mod args;
#[cfg(unix)]
mod encode;
#[cfg(unix)]
mod source;
#[cfg(unix)]
mod view;

// The build script, for its tests: its `main` and git calls run only when cargo builds.
#[cfg(test)]
#[path = "../build.rs"]
#[allow(dead_code)]
mod build;

use std::io::Write;
use std::process::ExitCode;

/// The command's name, as messages give it.
const NAME: &str = env!("CARGO_BIN_NAME");

// Keep stdout errors typed: a closed pipe is normal, but file and command errors still fail.
#[derive(Debug)]
enum CliError {
    Message(String),
    Stdout(std::io::Error),
}

impl From<String> for CliError {
    fn from(message: String) -> Self {
        Self::Message(message)
    }
}

impl From<&str> for CliError {
    fn from(message: &str) -> Self {
        Self::Message(message.to_string())
    }
}

impl std::fmt::Display for CliError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Message(message) => f.write_str(message),
            Self::Stdout(error) => write!(f, "stdout: {error}"),
        }
    }
}

fn stdout_line(line: std::fmt::Arguments<'_>) -> Result<(), CliError> {
    let mut stdout = std::io::stdout().lock();
    writeln!(stdout, "{line}").and_then(|()| stdout.flush()).map_err(CliError::Stdout)
}

fn main() -> ExitCode {
    let mut args: Vec<String> = Vec::new();
    for arg in std::env::args_os().skip(1) {
        match arg.into_string() {
            Ok(a) => args.push(a),
            Err(bad) => {
                let _ =
                    writeln!(std::io::stderr().lock(), "{NAME}: `{}` is not valid UTF-8, so it cannot be used as an argument", bad.to_string_lossy());
                return ExitCode::FAILURE;
            }
        }
    }
    let result = match args.first().map(String::as_str) {
        // Nothing given at all: what to give.
        #[cfg(unix)]
        None | Some("--help" | "-h") => view::print_help(source::BACKENDS),
        Some("--version" | "-V") => stdout_line(format_args!("{NAME} {}", env!("CLV_VERSION"))),
        // clv run again as a worker process by itself: not for the reader, so not in `--help`.
        #[cfg(unix)]
        Some("--worker") => source::serve(args.get(1).map_or("", String::as_str), source::WORKERS).map_err(CliError::Message),
        #[cfg(unix)]
        _ => view::run(&args, source::BACKENDS),
        #[cfg(not(unix))]
        _ => Err("needs a Unix terminal (macOS, Linux or FreeBSD)".into()),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(CliError::Stdout(e)) if e.kind() == std::io::ErrorKind::BrokenPipe => ExitCode::SUCCESS,
        Err(e) => {
            let _ = writeln!(std::io::stderr().lock(), "{NAME}: {e}");
            ExitCode::FAILURE
        }
    }
}
