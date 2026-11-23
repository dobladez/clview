//! `clv` as the tests run it: the one place that names the binary, so that no test reads the
//! developer's own config. `tests/it/fixtures.rs` can't hold this: the PDF backend's unit tests
//! share that file, and only an integration test knows where the binary is.

use std::path::PathBuf;
use std::process::Command;
use std::sync::OnceLock;

/// `clv` as built for these tests, to name in a shell line that [`script`] or a private tmux
/// server runs. Every other test runs it through [`clv`].
pub(crate) const CLV: &str = env!("CARGO_BIN_EXE_clv");

/// `clv`, kept away from the developer's own config ([`own_config`]).
pub(crate) fn clv() -> Command {
    let mut command = Command::new(CLV);
    own_config(&mut command);
    command
}

/// `command`, with `XDG_CONFIG_HOME` set to an empty directory, so that the `clv` it runs reads no
/// config: a `width = 72` of the developer's own would fail the tests. A test of the config sets
/// the variable again, to a directory of its own.
pub(crate) fn own_config(command: &mut Command) -> &mut Command {
    static EMPTY: OnceLock<PathBuf> = OnceLock::new();
    let empty = EMPTY.get_or_init(|| {
        let dir = std::env::temp_dir().join(format!("clview-no-config-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    });
    command.env("XDG_CONFIG_HOME", empty)
}

/// `line` run by a shell on a new pseudo-terminal: BSD script (macOS, FreeBSD) takes the command
/// after the output file; util-linux script (Linux) takes it as one shell line. What the line runs
/// gets its environment from script(1), so the `clv` in it reads no config ([`own_config`]); so
/// does a tmux server that the line starts, and the panes of that server.
pub(crate) fn script(line: &str) -> Command {
    let mut script = Command::new("script");
    if cfg!(target_os = "linux") {
        script.args(["-q", "-e", "-c", line, "/dev/null"]);
    } else {
        script.args(["-q", "/dev/null", "sh", "-c", line]);
    }
    own_config(&mut script);
    script
}

/// A directory to give `clv` as `XDG_CONFIG_HOME`, for a test of the config: it holds `text` as
/// `clv/config.toml`. One for each `test`.
pub(crate) fn config_home(test: &str, text: &str) -> PathBuf {
    let home = std::env::temp_dir().join(format!("clview-config-{test}-{}", std::process::id()));
    std::fs::create_dir_all(home.join("clv")).unwrap();
    std::fs::write(home.join("clv/config.toml"), text).unwrap();
    home
}
