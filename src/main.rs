//! clv: a file's pages shown in the terminal through the kitty graphics protocol.
//!
//! Usage: `clv --help`, which prints [`view::usage`].

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

#[cfg(unix)]
mod args;
#[cfg(unix)]
mod source;
#[cfg(unix)]
mod view;

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
        #[cfg(unix)]
        Some("--help" | "-h") => stdout_line(format_args!("{}", view::usage(source::BACKENDS).trim_end())),
        Some("--version" | "-V") => stdout_line(format_args!("{NAME} {}", env!("CARGO_PKG_VERSION"))),
        #[cfg(unix)]
        _ => view::run(&args),
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
