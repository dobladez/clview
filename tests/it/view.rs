//! `clv` draws pages through the kitty graphics protocol. Test output is a pipe, not a terminal,
//! so these tests pass `--protocol kitty` and read the escape codes back from stdout.
#![cfg(unix)]

use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use base64::Engine as _;

pub(crate) fn tmp(test: &str, name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("clview-{test}-{}", std::process::id()));
    std::fs::create_dir_all(&d).unwrap();
    d.join(name)
}

/// A US Letter document (612 × 792 pt) with `pages` pages.
fn fixture(test: &str, pages: usize) -> PathBuf {
    let pdf = tmp(test, if pages == 1 { "one.pdf" } else { "many.pdf" });
    std::fs::write(&pdf, crate::fixtures::letter("View test", pages)).unwrap();
    pdf
}

/// Run `clv`, kept away from the terminal, the tmux and the SSH login the tests may run in.
pub(crate) fn view(pdf: &Path, args: &[&str]) -> Output {
    view_with(pdf, args, &[])
}

/// [`view`], with `env` set.
pub(crate) fn view_with(pdf: &Path, args: &[&str], env: &[(&str, &str)]) -> Output {
    let mut command = kept_away();
    command.arg(pdf).args(args);
    command.envs(env.iter().copied()).output().unwrap()
}

/// `clv`, kept away as [`view`] says.
fn kept_away() -> Command {
    let mut command = crate::bin::clv();
    command.env("TERM", "dumb");
    for name in ["TMUX", "TMUX_PANE", "KITTY_WINDOW_ID", "TERM_PROGRAM", "SSH_CONNECTION", "SSH_CLIENT", "SSH_TTY", "COLORTERM", "NO_COLOR"] {
        command.env_remove(name);
    }
    command
}

pub(crate) fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

/// One image drawn by `clv`: its first control keys and its decoded payload.
pub(crate) struct Drawn {
    pub(crate) keys: String,
    pub(crate) png: Vec<u8>,
}

/// Split stdout into images: escape codes `ESC _G <control> ; <base64> ESC \`, with `m=1` on
/// every chunk but an image's last. Anything between images must be a newline, a carriage return,
/// a cursor move (`ESC [ n A`, `B` or `C`), or a line of a list's names ([`names`]).
pub(crate) fn images(stdout: &[u8]) -> Vec<Drawn> {
    printed(stdout).0
}

/// The lines of names under a list's rows of tiles, each with its runs of spaces made one.
pub(crate) fn names(stdout: &[u8]) -> Vec<String> {
    printed(stdout).1
}

/// [`images`], and the lines of names among them.
fn printed(stdout: &[u8]) -> (Vec<Drawn>, Vec<String>) {
    let mut rest = std::str::from_utf8(stdout).unwrap();
    let (mut out, mut keys, mut payload, mut names) = (Vec::new(), None::<String>, String::new(), Vec::new());
    while !rest.is_empty() {
        if keys.is_none() {
            if let Some(after) = rest.strip_prefix('\n').or_else(|| rest.strip_prefix('\r')) {
                rest = after;
                continue;
            }
            // Text to the end of its line, without an escape code.
            if !rest.starts_with('\x1b') {
                let (line, after) = rest.split_once('\n').unwrap_or((rest, ""));
                assert!(!line.contains('\x1b'), "an escape code in a line of names: {line:?}");
                names.push(line.split(' ').filter(|w| !w.is_empty()).fold(String::new(), |all, w| all + " " + w));
                rest = after;
                continue;
            }
            if let Some(after) = rest.strip_prefix("\x1b[") {
                let digits = after.find(|c: char| !c.is_ascii_digit()).unwrap();
                if digits > 0 && after[digits..].starts_with(['A', 'B', 'C']) {
                    rest = &after[digits + 1..];
                    continue;
                }
            }
        }
        let body = rest.strip_prefix("\x1b_G").unwrap_or_else(|| panic!("expected an escape code at {:?}", &rest[..rest.len().min(20)]));
        let end = body.find("\x1b\\").unwrap();
        let (control, data) = body[..end].split_once(';').unwrap();
        keys.get_or_insert_with(|| control.to_string());
        payload.push_str(data);
        if control.ends_with("m=0") {
            let png = base64::engine::general_purpose::STANDARD.decode(&payload).unwrap();
            out.push(Drawn { keys: keys.take().unwrap(), png });
            payload.clear();
        } else {
            assert!(control.ends_with("m=1"), "{control}");
        }
        rest = &body[end + 2..];
    }
    assert!(keys.is_none(), "an image was left unfinished");
    (out, names)
}

pub(crate) fn png_size(png: &[u8]) -> (u32, u32) {
    assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n");
    assert_eq!(&png[12..16], b"IHDR");
    let be32 = |b: &[u8]| u32::from_be_bytes([b[0], b[1], b[2], b[3]]);
    (be32(&png[16..20]), be32(&png[20..24]))
}

/// The value of `key` in a control string such as `a=T,c=40,r=20`.
fn key(control: &str, key: &str) -> u32 {
    control.split(',').find_map(|kv| kv.strip_prefix(key)?.strip_prefix('=')).unwrap().parse().unwrap()
}

#[test]
fn each_page_is_a_png_shown_at_the_cursor() {
    let pdf = fixture("pages", 2);
    let out = view(&pdf, &["--pages", "2,1", "--protocol", "kitty"]);
    assert!(out.status.success(), "{}", text(&out.stderr));
    let drawn = images(&out.stdout);
    assert_eq!(drawn.len(), 2);
    for image in &drawn {
        assert!(image.keys.starts_with("a=T,f=100,t=d,q=2,"), "{}", image.keys);
        let (w, h) = png_size(&image.png);
        // The page's shape, whatever the size of the terminal running the tests.
        let ratio = w as f32 / h as f32;
        assert!((ratio - 612.0 / 792.0).abs() < 0.01, "{w}×{h}");
        assert!(key(&image.keys, "c") >= 1 && key(&image.keys, "r") >= 1);
    }
}

#[test]
fn zoom_width_is_wider_than_a_page_to_a_row() {
    let pdf = fixture("fit", 1);
    let page = images(&view(&pdf, &["--protocol", "kitty", "--per-row", "1"]).stdout);
    let width = images(&view(&pdf, &["--protocol", "kitty", "--per-row", "width"]).stdout);
    let (cols_page, cols_width) = (key(&page[0].keys, "c"), key(&width[0].keys, "c"));
    // A portrait page fits the height of any window wider than it is tall in cells.
    assert!(cols_width >= cols_page, "{cols_width} < {cols_page}");
    assert!(png_size(&width[0].png).0 >= png_size(&page[0].png).0);
    // As the width, the page moves the cursor down and the screen with it, as text does: a page
    // taller than the window scrolls by.
    assert!(width[0].keys.starts_with("a=T,f=100,t=d,q=2,c="), "{}", width[0].keys);
    // Auto prints one page as one to a row does.
    let auto = images(&view(&pdf, &["--protocol", "kitty", "--per-row", "auto"]).stdout);
    assert_eq!((key(&auto[0].keys, "c"), png_size(&auto[0].png)), (cols_page, png_size(&page[0].png)));
    // Three to a row is a size, alone or not: a page in a third of the 80 columns, 25 of them.
    let third = images(&view(&pdf, &["--protocol", "kitty", "--per-row", "3"]).stdout);
    assert_eq!(key(&third[0].keys, "c"), 25, "{}", third[0].keys);
    assert!(png_size(&third[0].png).0 < png_size(&page[0].png).0);
}

/// `clv` on `pdf` with `args`, its config the file `text` in a directory of `test`'s own, and no home
/// to read another from: what it wrote, and what it said.
fn configured(test: &str, text: &str, pdf: &Path, args: &[&str]) -> Output {
    let home = crate::bin::config_home(test, text);
    view_with(pdf, args, &[("XDG_CONFIG_HOME", home.to_str().unwrap()), ("HOME", "/nonexistent")])
}

/// The config sets an option, when clv prints too: `per-row` there is `--per-row`. The same option
/// on the command line wins, `--no-config` reads no config, and `--config` reads another file.
/// Without `XDG_CONFIG_HOME`, the config is under `.config` in the home.
#[test]
fn config_sets_an_option_and_the_command_line_wins() {
    let pdf = fixture("config-option", 6);
    let cols = |out: &Output| {
        assert!(out.status.success() && out.stderr.is_empty(), "{}", text(&out.stderr));
        key(&images(&out.stdout)[0].keys, "c")
    };
    let typed = |args: &[&str]| cols(&view(&pdf, &[&["--protocol", "kitty"], args].concat()));
    let (auto, three, two) = (typed(&[]), typed(&["--per-row", "3"]), typed(&["--per-row", "2"]));
    assert!(three < two && three < auto, "{three} {two} {auto}");
    let config = |text: &str, args: &[&str]| cols(&configured("option", text, &pdf, &[&["--protocol", "kitty"], args].concat()));
    assert_eq!(config("# Mine.\nper-row = 3\n", &[]), three, "the config's");
    assert_eq!(config("per-row = \"3\"\n", &[]), three, "a number as a string");
    assert_eq!(config("per-row = 3\n", &["--per-row", "2"]), two, "the command line wins");
    assert_eq!(config("per-row = 3\n", &["--no-config"]), auto, "no config read");
    assert_eq!(config("", &[]), auto, "an empty config sets nothing");
    // What it prints with is the config's too.
    let out = configured("option", "protocol = \"kitty\"\nper-row = 3\n", &pdf, &[]);
    assert_eq!(cols(&out), three);
    // Another file, in place of the reader's own.
    let other = tmp("config-option", "other.toml");
    std::fs::write(&other, "per-row = 2\n").unwrap();
    assert_eq!(config("per-row = 3\n", &["--config", other.to_str().unwrap()]), two);
    // The reader's own, where `XDG_CONFIG_HOME` holds no absolute path: under `.config` in the home.
    let home = tmp("config-option", "home");
    std::fs::create_dir_all(home.join(".config/clv")).unwrap();
    std::fs::write(home.join(".config/clv/config.toml"), "per-row = 3\n").unwrap();
    for xdg in ["", "relative/config"] {
        let out = view_with(&pdf, &["--protocol", "kitty"], &[("XDG_CONFIG_HOME", xdg), ("HOME", home.to_str().unwrap())]);
        assert_eq!(cols(&out), three, "{xdg:?}");
    }
    // A flag set there holds, and one set to false is not given.
    let dir = tmp("config-option", "dir");
    std::fs::create_dir_all(dir.join("sub")).unwrap();
    std::fs::write(dir.join("a.pdf"), crate::fixtures::letter("A", 1)).unwrap();
    std::fs::write(dir.join("sub/b.pdf"), crate::fixtures::letter("B", 1)).unwrap();
    let listed = |text: &str| names(&configured("option", text, &dir, &["--protocol", "kitty", "--no-pager"]).stdout).join(" ");
    assert!(listed("recurse = false\n").contains("sub/") && !listed("recurse = false\n").contains("b.pdf"));
    assert!(listed("recurse = true\n").contains("sub/b.pdf"), "{}", listed("recurse = true\n"));
}

/// `pager` and `no-pager` are one option for the command line to win over: either typed wins over
/// both in the config. A config that sets both is refused.
#[test]
fn config_pager_and_no_pager_are_one_option_for_the_command_line() {
    let pdf = fixture("config-pager", 2);
    let needs = "clv: --pager needs a terminal to read keys from\n";
    let always = configured("pager", "pager = true\n", &pdf, &["--protocol", "kitty"]);
    assert!(!always.status.success() && text(&always.stderr) == needs, "{}", text(&always.stderr));
    let typed = configured("pager", "pager = true\n", &pdf, &["--protocol", "kitty", "--no-pager"]);
    assert!(typed.status.success() && typed.stderr.is_empty() && images(&typed.stdout).len() == 2, "{}", text(&typed.stderr));
    let other = configured("pager", "no-pager = true\n", &pdf, &["--protocol", "kitty", "--pager"]);
    assert!(!other.status.success() && text(&other.stderr) == needs, "{}", text(&other.stderr));
    let both = configured("pager", "pager = true\nrecurse = true\nno-pager = true\n", &pdf, &["--protocol", "kitty"]);
    assert!(!both.status.success() && both.stdout.is_empty());
    assert!(text(&both.stderr).ends_with("config.toml:3: pager and no-pager are both true: set one of them\n"), "{}", text(&both.stderr));
    // One of them false is one of them not given.
    let one = configured("pager", "pager = false\nno-pager = true\n", &pdf, &["--protocol", "kitty"]);
    assert!(one.status.success() && one.stderr.is_empty(), "{}", text(&one.stderr));
}

/// A config that is wrong stops clv before it shows anything, and the message names the file, the
/// line and the problem; text from the file is made safe for the terminal. A missing file is an
/// error only when it was asked for by name.
#[test]
fn config_that_is_wrong_stops_clv_and_names_the_file_and_the_line() {
    let pdf = fixture("config-wrong", 1);
    let file = crate::bin::config_home("wrong", "").join("clv/config.toml");
    for (written, said) in [
        ("compression = \"small\"\n\nper-row = 9\n", ":3: per-row: expected auto, width or 1–8, got 9"),
        ("protocol = \"sixel\"\n", ":1: protocol: expected auto, kitty, blocks or ascii, got sixel"),
        ("recurse = \"yes\"\n", ":1: recurse: expected true or false, got \"yes\""),
        ("# One.\nfrobnicate = 1\n", ":2: unknown option frobnicate"),
        ("pages = \"1\"\n", ":1: pages: a config can't set it: it belongs to one run"),
        ("password = \"x\"\n", ":1: password: a config can't set it: a password does not belong in a plain file"),
        ("config = \"x.toml\"\n", ":1: config: a config can't set it: it says which config to read"),
        ("zoom = 2\n", ":1: zoom is now per-row: write per-row"),
        ("compression = \"\\u001b]52;c;eA==\\u0007\"\n", ":1: compression: expected auto, fast or small, got \u{fffd}]52;c;eA==\u{fffd}"),
        ("\"na\\u001bme\" = 1\n", ":1: unknown option na\u{fffd}me"),
    ] {
        let out = configured("wrong", written, &pdf, &["--protocol", "kitty"]);
        assert!(!out.status.success() && out.stdout.is_empty(), "{written:?}");
        assert_eq!(text(&out.stderr), format!("clv: {}{said}\n", file.display()), "{written:?}");
    }
    // What is no TOML says its line too.
    let out = configured("wrong", "compression = \"small\"\nper-row = \n", &pdf, &["--protocol", "kitty"]);
    assert!(!out.status.success() && out.stdout.is_empty());
    assert!(text(&out.stderr).starts_with(&format!("clv: {}:2: ", file.display())), "{}", text(&out.stderr));
    // Named from the reader's home, as the reader writes it.
    let home = tmp("config-wrong", "home");
    std::fs::create_dir_all(home.join(".config/clv")).unwrap();
    std::fs::write(home.join(".config/clv/config.toml"), "compression = \"small\"\n\nper-row = 9\n").unwrap();
    let out = view_with(&pdf, &["--protocol", "kitty"], &[("XDG_CONFIG_HOME", ""), ("HOME", home.to_str().unwrap())]);
    assert_eq!(text(&out.stderr), "clv: ~/.config/clv/config.toml:3: per-row: expected auto, width or 1–8, got 9\n");
    // `--no-config` reads none, wrong or not.
    let out = configured("wrong", "per-row = 9\n", &pdf, &["--protocol", "kitty", "--no-config"]);
    assert!(out.status.success() && out.stderr.is_empty(), "{}", text(&out.stderr));
    // A file asked for by name must be there, and be a file within the size clv reads.
    let dir = tmp("config-wrong", "dir");
    std::fs::create_dir_all(&dir).unwrap();
    let large = tmp("config-wrong", "large.toml");
    std::fs::write(&large, vec![b'\n'; (1 << 20) + 1]).unwrap();
    let missing = tmp("config-wrong", "missing.toml");
    for (path, why) in [(&missing, "no such file"), (&dir, "not a regular file"), (&large, "larger than 1 MiB, the most of a config that clv reads")]
    {
        let out = configured("wrong", "", &pdf, &["--protocol", "kitty", "--config", path.to_str().unwrap()]);
        assert!(!out.status.success() && out.stdout.is_empty(), "{why}");
        assert_eq!(text(&out.stderr), format!("clv: {}: {why}\n", path.display()));
    }
    let out = configured("wrong", "", &pdf, &["--protocol", "kitty", "--config", missing.to_str().unwrap(), "--no-config"]);
    assert_eq!(text(&out.stderr), "clv: give --config or --no-config, not both\n");
    // A worker process reads no config: it answers as it does without one.
    let home = crate::bin::config_home("wrong", "per-row = 9\n");
    let worker = crate::bin::clv().args(["--worker", "none"]).env("XDG_CONFIG_HOME", &home).stdin(Stdio::null()).output().unwrap();
    assert!(!text(&worker.stderr).contains("config.toml"), "{}", text(&worker.stderr));
    let version = crate::bin::clv().arg("--version").env("XDG_CONFIG_HOME", &home).output().unwrap();
    assert!(version.status.success() && version.stderr.is_empty());
}

/// What a config can't bind stops clv before it shows anything, printing too, with the file and
/// the line: a command that doesn't apply in the keymap of its table, one that doesn't exist, a
/// digit, `ctrl-c`, and a key that a terminal sends as another.
#[test]
fn config_keys_that_cant_be_bound_are_refused_with_their_line() {
    let pdf = fixture("config-keys-wrong", 1);
    let file = crate::bin::config_home("keys-wrong", "").join("clv/config.toml");
    for (written, said) in [
        ("[keys]\nx = \"quit\"\n\n[keys.list]\ns = \"search\"\n", ":5: s: search does not apply in a list"),
        ("[keys.pages]\nx = \"list-open\"\n", ":2: x: list-open does not apply in a file's pages"),
        ("[keys]\nx = \"frobnicate\"\n", ":2: x: no command is called frobnicate"),
        ("[keys]\nx = \"paper\"\n", ":2: x: no command is called paper"),
        ("[keys]\n5 = \"quit\"\n", ":2: 5: a digit starts the number typed before a key, so it can't be bound"),
        ("[keys]\nctrl-c = \"none\"\n", ":2: ctrl-c: it always quits, so it can't be bound"),
        ("[keys]\nctrl-D = \"screen-down\"\n", ":2: ctrl-D: write ctrl-d: a terminal sends the same byte for both"),
        ("[keys]\nctrl-m = \"next\"\n", ":2: ctrl-m: write enter: a terminal sends the same byte for both"),
        ("[keys]\nctrl-h = \"back\"\n", ":2: ctrl-h: write backspace: a terminal sends the same byte for both"),
        ("[keys]\ntab = \"next\"\n", ":2: tab: clv does not read Tab as a key"),
        ("[keys]\nctrl-i = \"next\"\n", ":2: ctrl-i: a terminal sends the same byte for ctrl-i and Tab, and clv does not read Tab as a key"),
        ("[keys]\nEnter = \"next\"\n", ":2: Enter: write enter: the names of keys are in lower case"),
        ("[keys]\nx = 3\n", ":2: x: expected the name of a command or \"none\", got 3"),
        // A key above the tables, and an option below one: each says where its line goes.
        ("per-row = 3\nx = \"quit\"\n\n[keys]\n", ":2: unknown option x: x is a key, and a key is bound under [keys]"),
        ("ctrl-d = \"screen-down\"\n", ":1: unknown option ctrl-d: ctrl-d is a key, and a key is bound under [keys]"),
        (
            "[keys]\n\n[keys.list]\nJ = \"list-tile-down\"\nper-row = 3\n",
            ":5: per-row is an option, not a key: write it above [keys], before the first table",
        ),
        ("[keys]\nrecurse = true\n", ":2: recurse is an option, not a key: write it above [keys], before the first table"),
        ("keys = \"x\"\n", ":1: keys: expected a table of keys"),
        ("[keys]\n\"\\u001b]0;t\\u0007\" = \"quit\"\n", ":2: \u{fffd}]0;t\u{fffd}: clv reads no key of that name"),
        ("[keys]\nx = \"qu\\u001bit\"\n", ":2: x: no command is called qu\u{fffd}it"),
    ] {
        let out = configured("keys-wrong", written, &pdf, &["--protocol", "kitty"]);
        assert!(!out.status.success() && out.stdout.is_empty(), "{written:?}");
        assert_eq!(text(&out.stderr), format!("clv: {}{said}\n", file.display()), "{written:?}");
    }
    // Keys that can be bound say nothing, and change nothing that is printed.
    let bound = "[keys]\nx = \"quit\"\n\"/\" = \"none\"\nctrl-d = \"screen-down\"\n\n[keys.list]\nJ = \"list-tile-down\"\n";
    let out = configured("keys-wrong", bound, &pdf, &["--protocol", "kitty"]);
    assert!(out.status.success() && out.stderr.is_empty(), "{}", text(&out.stderr));
    assert_eq!(out.stdout, view(&pdf, &["--protocol", "kitty"]).stdout);
}

/// `clv` with `args` and no file, its config under `home`, and no home to read another from.
fn clv_with_config_home(home: &Path, args: &[&str]) -> Output {
    crate::bin::clv().args(args).env("XDG_CONFIG_HOME", home).env("HOME", "/nonexistent").env("TERM", "dumb").output().unwrap()
}

/// `--generate-config` writes the template of a config where clv reads one, makes the directory,
/// and prints the path. Every line that sets something is commented out, so the file sets nothing. A second
/// time it refuses, names the file, and leaves it as it was. `--no-config --print-config` prints
/// what it wrote. With `--config FILE`, it writes that file.
#[test]
fn generate_config_writes_the_template_once_and_print_config_prints_it() {
    let home = tmp("generate-config", "xdg");
    let _ = std::fs::remove_dir_all(&home);
    let file = home.join("clv/config.toml");
    let run = |args: &[&str]| clv_with_config_home(&home, args);
    let out = run(&["--generate-config"]);
    assert!(out.status.success() && out.stderr.is_empty(), "{}", text(&out.stderr));
    assert_eq!(text(&out.stdout), format!("{}\n", file.display()), "the path it wrote");
    let written = std::fs::read_to_string(&file).unwrap();
    // Every line that sets something is commented out; the three tables of the keys are as they are.
    let tables = ["[keys]", "[keys.pages]", "[keys.list]"];
    assert!(written.lines().all(|line| line.is_empty() || line.starts_with("##") || line.starts_with("# ") || tables.contains(&line)), "{written}");
    // It starts by saying how to read it.
    assert!(written.starts_with("## The config of clv. clv reads this file when it starts, and never changes it.\n##\n"), "{written}");
    assert!(written.contains("\n## A line that starts with one \"#\" is an example: it shows the default.\n"), "{written}");
    assert!(tables.iter().all(|table| written.lines().filter(|line| line == table).count() == 1), "{written}");
    // A key that both keymaps bind alike is there once.
    assert_eq!(written.lines().filter(|line| *line == "# q = \"quit\"").count(), 1, "{written}");
    for line in [
        "# per-row = \"auto\"",
        "# recurse = false",
        "[keys]",
        "[keys.pages]",
        "[keys.list]",
        "# q = \"quit\"",
        "# enter = \"list-open\"",
        "# \"/\" = \"search\"",
    ] {
        assert!(written.lines().any(|other| other == line), "{line}:\n{written}");
    }
    assert!(
        written.contains("## A config cannot change two bindings: ctrl-c always quits, and a number\n## before enter goes to that page or tile.\n"),
        "{written}"
    );
    assert!(!written.contains("password") && !written.contains("pages ="), "{written}");
    // clv reads it, and it sets nothing.
    let pdf = fixture("generate-config", 2);
    let env = [("XDG_CONFIG_HOME", home.to_str().unwrap())];
    let read = view_with(&pdf, &["--protocol", "kitty"], &env);
    assert!(read.status.success() && read.stderr.is_empty(), "{}", text(&read.stderr));
    assert_eq!(read.stdout, view(&pdf, &["--protocol", "kitty"]).stdout);
    // The second time: refused, and the file is as it was.
    std::fs::write(&file, "# Mine.\nper-row = 3\n").unwrap();
    let again = run(&["--generate-config"]);
    assert!(!again.status.success() && again.stdout.is_empty());
    assert_eq!(text(&again.stderr), format!("clv: {} is there already: clv never writes over a config\n", file.display()));
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "# Mine.\nper-row = 3\n");
    // What it wrote is the config in effect with no config.
    let printed = run(&["--no-config", "--print-config"]);
    assert!(printed.status.success() && printed.stderr.is_empty(), "{}", text(&printed.stderr));
    assert_eq!(text(&printed.stdout), written);
    // Another file, its directories made.
    let other = tmp("generate-config", "other/deep/mine.toml");
    let _ = std::fs::remove_dir_all(other.parent().unwrap());
    let out = run(&["--generate-config", "--config", other.to_str().unwrap()]);
    assert!(out.status.success() && text(&out.stdout) == format!("{}\n", other.display()), "{}", text(&out.stderr));
    assert_eq!(std::fs::read_to_string(&other).unwrap(), written);
    // The path it prints is whole, for a file named from where clv runs too.
    let here = tmp("generate-config", "here");
    let _ = std::fs::remove_dir_all(&here);
    std::fs::create_dir_all(&here).unwrap();
    for named in ["mine.toml", "./sub/mine.toml"] {
        let out = crate::bin::clv().args(["--generate-config", "--config", named]).current_dir(&here).output().unwrap();
        assert!(out.status.success(), "{named}: {}", text(&out.stderr));
        let printed = PathBuf::from(text(&out.stdout).trim_end_matches('\n'));
        assert!(printed.is_absolute() && !printed.components().any(|part| part.as_os_str() == "."), "{named}: {}", printed.display());
        assert_eq!(printed.canonicalize().unwrap(), here.join(named).canonicalize().unwrap(), "{named}");
        assert_eq!(std::fs::read_to_string(&printed).unwrap(), written);
    }
    // Each is the whole of a run.
    for (args, said) in [
        (&["--generate-config", "a.pdf"][..], "clv: --generate-config takes no file\n"),
        (&["--print-config", "a.pdf"], "clv: --print-config takes no file\n"),
        (&["--generate-config", "--print-config"], "clv: give --generate-config or --print-config, not both\n"),
    ] {
        let out = run(args);
        assert!(!out.status.success() && out.stdout.is_empty() && text(&out.stderr) == said, "{args:?}: {}", text(&out.stderr));
    }
}

/// `--print-config` prints the config in effect, from the file and the command line: a line that
/// either sets has its value and no `# `, the others their defaults, commented out. The command
/// line wins. What it prints is a config that clv takes, and that prints the same.
#[test]
fn print_config_prints_what_the_file_and_the_command_line_set() {
    let home = crate::bin::config_home("print", "compression = \"small\"\nper-row = 3\npager = true\n\n[keys]\nx = \"quit\"\nq = \"none\"\n");
    let printed = |args: &[&str]| {
        let out = clv_with_config_home(&home, args);
        assert!(out.status.success() && out.stderr.is_empty(), "{args:?}: {}", text(&out.stderr));
        text(&out.stdout)
    };
    let once = |printed: &str, line: &str| printed.lines().filter(|other| *other == line).count() == 1;
    let file = printed(&["--print-config"]);
    for line in [
        "compression = \"small\"",
        "per-row = 3",
        "pager = true",
        "# no-pager = false",
        "# recurse = false",
        "# protocol = \"auto\"",
        "[keys.pages]",
        "[keys.list]",
    ] {
        assert!(once(&file, line), "{line}:\n{file}");
    }
    // Bound alike in both keymaps: written once, under `[keys]`.
    assert_eq!(file.matches("\nx = \"quit\"\nq = \"none\"\n").count(), 1, "once, for both keymaps:\n{file}");
    assert!(file.find("\nx = \"quit\"\n") < file.find("\n[keys.pages]\n") && file.find("\n[keys]\n") < file.find("\nx = \"quit\"\n"), "{file}");
    assert!(!file.contains("# q = \"quit\""), "{file}");
    // The command line's, and the command line wins.
    let typed = printed(&["--print-config", "--per-row", "2", "--recurse", "--protocol", "kitty", "--no-pager"]);
    for line in ["compression = \"small\"", "per-row = 2", "recurse = true", "protocol = \"kitty\"", "no-pager = true", "# pager = false"] {
        assert!(once(&typed, line), "{line}:\n{typed}");
    }
    assert!(!typed.contains("per-row = 3"), "{typed}");
    // What it prints is a config: read as one, it prints the same.
    let again = tmp("print-config", "again.toml");
    std::fs::write(&again, &file).unwrap();
    assert_eq!(printed(&["--print-config", "--config", again.to_str().unwrap()]), file);
    // A config that is wrong is said, not printed.
    let wrong = crate::bin::config_home("print-wrong", "per-row = 9\n");
    let out = clv_with_config_home(&wrong, &["--print-config"]);
    assert!(
        !out.status.success() && out.stdout.is_empty() && text(&out.stderr).contains("config.toml:1: per-row: expected auto, width or 1–8, got 9")
    );
}

/// `--print-config` checks the options as a run does, before it prints them: a value that an
/// option doesn't take, and `--pager` with `--no-pager`, are refused with the run's own message,
/// and nothing is printed. So what it prints is always a config that clv takes.
#[test]
fn print_config_refuses_what_a_run_refuses_and_prints_nothing() {
    let home = crate::bin::config_home("print-refuses", "");
    for (args, said) in [
        (&["--protocol", "sixel"][..], "clv: --protocol: expected auto, kitty, blocks or ascii, got sixel\n"),
        (&["--compression", "best"], "clv: --compression: expected auto, fast or small, got best\n"),
        (&["--per-row", "9"], "clv: --per-row: expected auto, width or 1–8, got 9\n"),
        (&["--per-row", "0x3"], "clv: --per-row: expected auto, width or 1–8, got 0x3\n"),
        (&["--pager", "--no-pager"], "clv: give --pager or --no-pager, not both\n"),
        (&["--pages", "0"], "clv: --pages: bad page number \"0\"\n"),
    ] {
        for which in [&["--no-config", "--print-config"][..], &["--print-config"]] {
            let out = clv_with_config_home(&home, &[which, args].concat());
            assert!(!out.status.success() && out.stdout.is_empty(), "{args:?}: {}", text(&out.stdout));
            assert_eq!(text(&out.stderr), said, "{args:?}");
            // As the run itself says it.
            let run = view(&fixture("print-refuses", 1), &[&["--protocol", "kitty"], args].concat());
            assert!(args.contains(&"--protocol") || text(&run.stderr) == said, "{args:?}: {}", text(&run.stderr));
        }
    }
    // The command line's `--pager` over the config's `no-pager` is no clash: it wins.
    let home = crate::bin::config_home("print-refuses", "no-pager = true\n");
    let out = clv_with_config_home(&home, &["--print-config", "--pager"]);
    assert!(out.status.success() && text(&out.stdout).contains("\npager = true\n") && text(&out.stdout).contains("\n# no-pager = false\n"));
}

/// `--print-config` writes a number as clv read it, in base ten, never as it was typed: `03` and
/// `+3` are `3`. So a number that TOML would refuse as written (a leading zero) still prints as a
/// config that clv takes, and that prints the same.
#[test]
fn print_config_writes_a_number_as_it_was_read_not_as_it_was_typed() {
    let printed = |config: &str, args: &[&str]| {
        let out = clv_with_config_home(&crate::bin::config_home("print-number", config), &[&["--print-config"], args].concat());
        assert!(out.status.success() && out.stderr.is_empty(), "{config:?} {args:?}: {}", text(&out.stderr));
        text(&out.stdout)
    };
    let as_three = printed("", &["--per-row", "3"]);
    assert!(as_three.lines().any(|line| line == "per-row = 3"), "{as_three}");
    for (config, args) in [
        ("", &["--per-row", "03"][..]),
        ("", &["--per-row", "+3"]),
        ("", &["--per-row", "0003"]),
        ("per-row = \"03\"\n", &[]),
        ("per-row = \"+3\"\n", &[]),
        ("per-row = 0x3\n", &[]),
        ("per-row = 0o3\n", &[]),
        ("per-row = 3\n", &[]),
    ] {
        let out = printed(config, args);
        assert_eq!(out, as_three, "{config:?} {args:?}");
        // What it printed reads back, and prints the same.
        let again = tmp("print-number", "again.toml");
        std::fs::write(&again, &out).unwrap();
        assert_eq!(printed("", &["--config", again.to_str().unwrap()]), out, "{config:?} {args:?}");
    }
}

#[test]
fn without_a_terminal_auto_points_to_the_protocol_option() {
    let pdf = fixture("auto", 1);
    let out = view(&pdf, &[]);
    assert!(!out.status.success());
    assert!(out.stdout.is_empty());
    let stderr = text(&out.stderr);
    assert!(stderr.contains("stdout is not a terminal") && stderr.contains("--protocol kitty"), "{stderr}");
}

#[test]
fn page_numbers_are_checked_before_anything_is_drawn() {
    let pdf = fixture("past", 1);
    let out = view(&pdf, &["--pages", "1,3", "--protocol", "kitty"]);
    assert!(!out.status.success());
    assert!(out.stdout.is_empty());
    assert!(text(&out.stderr).contains("no page 3"), "{}", text(&out.stderr));
}

/// `--zoom` is `--per-row`'s old name: still taken, with the same values, and a warning once, before
/// anything else; given with `--per-row`, refused; its errors name it as typed.
#[test]
fn zoom_is_still_taken_for_per_row_and_warns() {
    let pdf = fixture("zoom-alias", 3);
    let old = view(&pdf, &["--protocol", "kitty", "--zoom", "3"]);
    let new = view(&pdf, &["--protocol", "kitty", "--per-row", "3"]);
    assert!(old.status.success() && new.status.success(), "{}", text(&old.stderr));
    assert_eq!(old.stdout, new.stdout, "the same pages, three to a row");
    assert_eq!(text(&old.stderr), "warning: --zoom is now --per-row; --zoom will go in a later release\n");
    assert_eq!(text(&new.stderr), "", "no warning by its own name");
    // Before anything else: an error after it, the warning still first.
    let out = crate::bin::clv().args(["--zoom", "2"]).output().unwrap();
    assert_eq!(text(&out.stderr), "warning: --zoom is now --per-row; --zoom will go in a later release\nclv: missing file\n");
    // Both: refused, before anything is drawn.
    let both = view(&pdf, &["--protocol", "kitty", "--zoom", "2", "--per-row", "3"]);
    assert!(!both.status.success() && both.stdout.is_empty());
    assert!(text(&both.stderr).contains("--per-row and --zoom are one option: give one of them"), "{}", text(&both.stderr));
    // An error names the option as it was typed.
    for (option, said) in [("--zoom", "clv: --zoom: expected auto"), ("--per-row", "clv: --per-row: expected auto")] {
        let out = view(&pdf, &["--protocol", "kitty", option, "9"]);
        assert!(!out.status.success() && text(&out.stderr).contains(said), "{option}: {}", text(&out.stderr));
    }
}

#[test]
fn bad_arguments_say_what_is_allowed() {
    let pdf = fixture("args", 1);
    for (args, expected) in [
        // One form of message for a value an option doesn't take, the viewer's and a setting's alike.
        (&["--per-row", "sideways"][..], "clv: --per-row: expected auto, width or 1–8, got sideways\n"),
        (&["--per-row", "9"], "clv: --per-row: expected auto, width or 1–8, got 9\n"),
        (&["--protocol", "sixel"], "clv: --protocol: expected auto, kitty, blocks or ascii, got sixel\n"),
        (&["--compression", "best"], "clv: --compression: expected auto, fast or small, got best\n"),
        (&["--per-row", "\x1b[2J"], "clv: --per-row: expected auto, width or 1–8, got \u{fffd}[2J\n"),
        (&["--fit", "width"], "unknown option --fit"),
        (&["--pages", "0"], "bad page number"),
        (&["--protocol", "kitty", "--pages"], "--pages needs a value"),
        (&["--protocol", "kitty", "--pager", "--no-pager"], "not both"),
    ] {
        let out = view(&pdf, args);
        assert!(!out.status.success(), "{args:?}");
        assert!(text(&out.stderr).contains(expected), "{args:?}: {}", text(&out.stderr));
    }
}

/// `--theme` is the documents' backend's setting: a build without them doesn't know it.
#[cfg(not(feature = "word"))]
#[test]
fn without_documents_there_is_no_theme() {
    let out = view(&fixture("no-theme", 1), &["--protocol", "kitty", "--theme", "dark"]);
    assert!(!out.status.success() && text(&out.stderr).contains("unknown option --theme"), "{}", text(&out.stderr));
}

/// Nothing given at all is a call for help: what `--help` prints, and no error.
#[test]
fn no_arguments_print_the_help() {
    let help = crate::bin::clv().arg("--help").output().unwrap();
    let out = crate::bin::clv().output().unwrap();
    assert!(out.status.success() && out.stderr.is_empty(), "{}", text(&out.stderr));
    assert_eq!(text(&out.stdout), text(&help.stdout));
}

/// Run `clv` as if in a tmux pane, with no `tmux` command on the PATH to ask.
fn view_in_tmux(test: &str, pdf: &Path, args: &[&str]) -> Output {
    let empty = tmp(test, "empty-bin");
    std::fs::create_dir_all(&empty).unwrap();
    crate::bin::clv()
        .arg(pdf)
        .args(args)
        .env("TMUX", "/nonexistent/tmux-socket,1,0")
        .env_remove("TMUX_PANE")
        .env_remove("KITTY_WINDOW_ID")
        .env_remove("TERM_PROGRAM")
        .env("TERM", "tmux-256color")
        .env("PATH", &empty)
        .output()
        .unwrap()
}

/// The bytes inside the tmux passthrough sequences at the start of `bytes` (`ESC P tmux; …
/// ESC \`, with every inner ESC doubled), and what follows them.
fn passthrough(bytes: &[u8]) -> (Vec<u8>, &[u8]) {
    let mut inner = Vec::new();
    let mut rest = bytes;
    while let Some(body) = rest.strip_prefix(b"\x1bPtmux;") {
        let mut i = 0;
        loop {
            match (body[i], body[i + 1]) {
                (0x1b, 0x1b) => {
                    inner.push(0x1b);
                    i += 2;
                }
                (0x1b, b'\\') => {
                    i += 2;
                    break;
                }
                (b, _) => {
                    assert_ne!(b, 0x1b, "a lone ESC inside a passthrough sequence");
                    inner.push(b);
                    i += 1;
                }
            }
        }
        rest = &body[i..];
    }
    (inner, rest)
}

#[test]
fn inside_tmux_pages_go_through_passthrough_and_placeholders_draw_them() {
    let pdf = fixture("tmux", 1);
    let out = view_in_tmux("tmux", &pdf, &["--protocol", "kitty"]);
    assert!(out.status.success(), "{}", text(&out.stderr));
    // With no tmux to ask, `clv` says it couldn't check passthrough, and goes on.
    assert!(text(&out.stderr).contains("allow-passthrough"), "{}", text(&out.stderr));

    // The image is transmitted, then placed virtually, all through passthrough.
    let (codes, after) = passthrough(&out.stdout);
    let mut transmitted: Vec<(&str, &str)> = std::str::from_utf8(&codes)
        .unwrap()
        .strip_suffix("\x1b\\")
        .unwrap()
        .split("\x1b\\")
        .map(|code| {
            let body = code.strip_prefix("\x1b_G").unwrap();
            body.split_once(';').unwrap_or((body, ""))
        })
        .collect();
    let (place, _) = transmitted.pop().unwrap();
    let id = key(transmitted[0].0, "i");
    assert!(transmitted[0].0.starts_with("a=t,f=100,t=d,q=2,i="), "{}", transmitted[0].0);
    // tmux couldn't say whether it passes 24-bit colour, so a palette colour carries the low 8
    // bits and a third diacritic the high byte; nothing is left for the bits in between.
    assert_eq!(id & 0x00FF_FF00, 0, "{id:#x}");
    assert_ne!(id & 0xff, 0, "{id:#x}");
    let png = base64::engine::general_purpose::STANDARD.decode(transmitted.iter().map(|(_, p)| *p).collect::<String>()).unwrap();
    let (w, h) = png_size(&png);
    assert!((w as f32 / h as f32 - 612.0 / 792.0).abs() < 0.01, "{w}×{h}");
    let (cols, rows) = (key(place, "c"), key(place, "r"));
    assert_eq!(place, format!("a=p,U=1,q=2,i={id},c={cols},r={rows}"));

    // Then a line of placeholder cells per row, in the colour that names the image.
    let lines: Vec<&str> = std::str::from_utf8(after).unwrap().split_terminator('\n').collect();
    assert_eq!(lines.len(), rows as usize);
    for line in lines {
        assert!(line.starts_with(&format!("\x1b[38;5;{}m", id & 0xff)) && line.ends_with("\x1b[39m"), "{line:?}");
        assert_eq!(line.chars().filter(|c| *c == '\u{10EEEE}').count(), cols as usize);
    }
}

/// Printed inside tmux, pages side by side share their lines of placeholder cells: the first
/// page's, the gap, then the second's, each in the colour that names its image.
#[test]
fn inside_tmux_pages_side_by_side_share_their_lines() {
    let pdf = fixture("tmux-zoom", 2);
    let out = view_in_tmux("tmux-zoom", &pdf, &["--protocol", "kitty", "--per-row", "2"]);
    assert!(out.status.success(), "{}", text(&out.stderr));
    let (codes, after) = passthrough(&out.stdout);
    let codes = std::str::from_utf8(&codes).unwrap();
    let placed: Vec<&str> = codes.split("\x1b\\").filter_map(|code| code.strip_prefix("\x1b_Ga=p,U=1,")).collect();
    assert_eq!(placed.len(), 2, "{codes:?}");
    let (first, second) = (key(placed[0], "i"), key(placed[1], "i"));
    assert_ne!(first, second);
    let (cols, rows) = (key(placed[0], "c"), key(placed[0], "r"));
    let lines: Vec<&str> = std::str::from_utf8(after).unwrap().split_terminator('\n').collect();
    assert_eq!(lines.len(), rows as usize);
    for line in lines {
        let (left, right) = line.split_once("\x1b[39m  ").unwrap_or_else(|| panic!("no gap between the pages: {line:?}"));
        assert!(left.starts_with(&format!("\x1b[38;5;{}m", first & 0xff)), "{line:?}");
        assert!(right.starts_with(&format!("\x1b[38;5;{}m", second & 0xff)) && right.ends_with("\x1b[39m"), "{line:?}");
        assert_eq!(line.chars().filter(|c| *c == '\u{10EEEE}').count(), 2 * cols as usize);
    }
}

#[test]
fn inside_tmux_auto_still_needs_a_terminal() {
    let pdf = fixture("tmux-auto", 1);
    let out = view_in_tmux("tmux-auto", &pdf, &[]);
    assert!(!out.status.success());
    assert!(out.stdout.is_empty());
    assert!(text(&out.stderr).contains("stdout is not a terminal"), "{}", text(&out.stderr));
}

#[test]
fn a_hostile_file_name_cant_send_escape_sequences_to_the_terminal() {
    // A name that would write to the clipboard (OSC 52) and retitle the window if printed raw.
    let dir = tmp("escape", "");
    let name = dir.join("x\x1b]52;c;cHduZWQ=\x07y\x1b]0;owned\x07.pdf");
    for args in [&[][..], &["--protocol", "kitty"], &["--pages", "x\x1b[2J"]] {
        let out = crate::bin::clv().arg(&name).args(args).env_remove("TMUX").output().unwrap();
        assert!(!out.status.success());
        let stderr = text(&out.stderr);
        // A bad page list is said, quoted with its escapes spelled out, before any file is read.
        let said = if args.len() == 2 && args[0] == "--pages" { r"x\u{1b}[2J" } else { "x\u{FFFD}]52;c;cHduZWQ=\u{FFFD}y" };
        assert!(stderr.contains(said), "{stderr:?}");
        assert!(!out.stderr.iter().any(|b| *b == 0x1b || *b == 0x07), "{stderr:?}");
    }
}

#[test]
fn compression_trades_bytes_for_speed_and_auto_follows_ssh() {
    let pdf = fixture("compression", 1);
    let png = |args: &[&str], env: &[(&str, &str)]| {
        let out = view_with(&pdf, &[&["--protocol", "kitty"], args].concat(), env);
        assert!(out.status.success(), "{}", text(&out.stderr));
        images(&out.stdout).remove(0).png
    };
    let (fast, small) = (png(&["--compression", "fast"], &[]), png(&["--compression", "small"], &[]));
    // The same page, in more bytes for less work.
    assert_eq!(png_size(&fast), png_size(&small));
    assert!(fast.len() > small.len(), "fast {} bytes, small {}", fast.len(), small.len());
    // auto: fast on this machine, small when the login came over SSH.
    assert_eq!(png(&[], &[]), fast);
    assert_eq!(png(&[], &[("SSH_CONNECTION", "192.0.2.7 50122 192.0.2.1 22")]), small);
    assert_eq!(png(&["--compression", "fast"], &[("SSH_TTY", "/dev/pts/3")]), fast);
    let bad = view(&pdf, &["--protocol", "kitty", "--compression", "best"]);
    assert!(text(&bad.stderr).contains("--compression: expected auto, fast or small, got best"));
}

#[test]
fn help_and_version() {
    let out = crate::bin::clv().arg("--help").output().unwrap();
    let help = text(&out.stdout);
    let is = "Command-line Viewer: view graphic files without leaving your terminal";
    let start = format!("clview {}\n{is}\n\nclv [options] <file>…\nclv --help | --version\n", env!("CLV_VERSION"));
    assert!(out.status.success() && help.starts_with(&start), "{help}");
    assert!(help.contains("\n\nGeneral Options:\n  --pages ") && help.contains("\n  --per-row auto|width|1-8 "), "{help}");
    assert!(!help.contains("--zoom"), "the old name is not shown: {help}");
    assert!(help.contains("\n  -r, --recurse "), "{help}");
    // A backend that takes options of its own lists them under a heading of its own, what it
    // shows: the PDF one, and the documents', the spreadsheets', Mermaid's, the books' and Org's when
    // they are built.
    let heading = |(line, next): &(&str, &str)| line.starts_with("   for ") && line.ends_with(':') && next.starts_with("     --");
    let headings = help.lines().zip(help.lines().skip(1)).filter(heading).count();
    let built = [cfg!(feature = "word"), cfg!(feature = "sheet"), cfg!(feature = "mermaid"), cfg!(feature = "epub"), cfg!(feature = "org")];
    assert_eq!(headings, 1 + built.iter().filter(|built| **built).count(), "{help}");
    assert!(help.contains("\n\n   for PDFs:\n     --password PW ") && !help.contains("no options"), "{help}");
    // A setting's option is its backends': `--theme` the documents' and Mermaid's, which share the
    // setting, listed under each in its own words.
    assert!(!help.contains("\n  --theme "), "{help}");
    let under = |shows: &str| {
        let options = help.split(&format!("\n   for {shows}:\n")).nth(1).map(|rest| rest.split("\n\n").next().unwrap_or_default());
        let theme = options.and_then(|options| options.lines().find(|line| line.starts_with("     --theme auto|light|dark ")));
        theme.map(|line| line.trim_start_matches("     --theme auto|light|dark").trim().to_owned())
    };
    let (documents, diagrams) = (under("documents"), under("Mermaid diagrams"));
    assert_eq!(documents.as_ref().is_some_and(|help| help.starts_with("Markdown and HTML ")), cfg!(feature = "word"), "{help}");
    assert_eq!(diagrams.as_ref().is_some_and(|help| help.contains("Mermaid's dark theme")), cfg!(feature = "mermaid"), "{help}");
    assert!(documents != diagrams || documents.is_none(), "each in its own words: {help}");
    // Then the formats shown, by the libraries each backend credits: a backend's only when it is
    // built.
    let shown = |what: &str, formats: &str| {
        let row =
            |line: &str| line.strip_prefix(&format!("  {what}, by ")).and_then(|rest| rest.split_once(": ")).map(|(_, listed)| listed.to_owned());
        help.lines().filter_map(row).any(|listed| listed.starts_with(formats))
    };
    assert!(shown("PDFs", "PDF"), "{help}");
    assert_eq!(shown("Images", "PNG, JPEG, "), cfg!(feature = "image"), "{help}");
    assert_eq!(shown("SVG drawings", "SVG (.svg), svgz (.svgz)"), cfg!(feature = "svg"), "{help}");
    // With the language of the fenced blocks a document shows as pictures of it.
    assert_eq!(shown("Mermaid diagrams", "Mermaid (.mmd .mermaid, ```mermaid)"), cfg!(feature = "mermaid"), "{help}");
    assert_eq!(shown("Zip archives", "zip"), cfg!(feature = "zip"), "{help}");
    assert_eq!(shown("Tar archives and compressed files", "tar, gzip, "), cfg!(feature = "tar"), "{help}");
    assert_eq!(shown("Documents", "Word (.docx "), cfg!(feature = "word"), "{help}");
    assert_eq!(shown("Spreadsheets", "Excel (.xlsx "), cfg!(feature = "sheet"), "{help}");
    assert_eq!(shown("Ebooks", "EPUB (.epub)"), cfg!(feature = "epub"), "{help}");
    assert_eq!(shown("Org documents", "Org (.org .org_archive)"), cfg!(feature = "org"), "{help}");
    let out = crate::bin::clv().arg("--version").output().unwrap();
    assert_eq!(text(&out.stdout), format!("clv {}\n", env!("CLV_VERSION")));
    // Cargo.toml's version, alone or followed by the commit as build metadata (build.rs).
    let commit = env!("CLV_VERSION").strip_prefix(env!("CARGO_PKG_VERSION"));
    assert!(commit.is_some_and(|commit| commit.is_empty() || commit.starts_with('+')), "{}", env!("CLV_VERSION"));
}

/// A file is shown by the first backend that opens it, going by its first bytes, not its name.
#[test]
fn a_file_no_backend_opens_is_refused_and_a_pdf_is_known_by_its_header() {
    let notes = tmp("backends", "notes.pdf");
    std::fs::write(&notes, "not a PDF, whatever its name says\n").unwrap();
    let out = view(&notes, &["--protocol", "kitty"]);
    assert_eq!(text(&out.stderr), format!("clv: {}: not a file clv shows (it shows {})\n", notes.display(), shows()));
    assert!(!out.status.success() && out.stdout.is_empty());
    let pdf = tmp("backends", "page");
    std::fs::write(&pdf, crate::fixtures::letter("No extension", 1)).unwrap();
    let out = view(&pdf, &["--protocol", "kitty"]);
    assert!(out.status.success(), "{}", text(&out.stderr));
    assert_eq!(images(&out.stdout).len(), 1);
}

/// The file is read once, so a PDF can come through a pipe: the bytes read to choose its backend
/// are not lost to it.
#[test]
fn a_pdf_through_a_pipe_is_shown() {
    use std::io::Write as _;
    let mut child = crate::bin::clv()
        .args(["/dev/stdin", "--protocol", "kitty"])
        .env("TERM", "dumb")
        .env_remove("TMUX")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(&crate::fixtures::letter("Piped", 2)).unwrap();
    let out = child.wait_with_output().unwrap();
    assert!(out.status.success(), "{}", text(&out.stderr));
    assert_eq!(images(&out.stdout).len(), 2);
}

/// A picture through a pipe can't be read again when it renders, so it is kept as it came.
#[cfg(feature = "image")]
#[test]
fn a_picture_through_a_pipe_is_shown() {
    use std::io::Write as _;
    let png = std::fs::read(picture("piped", "photo.png", 30, 20)).unwrap();
    let mut child = crate::bin::clv()
        .args(["/dev/stdin", "--protocol", "kitty"])
        .env("TERM", "dumb")
        .env_remove("TMUX")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(&png).unwrap();
    let out = child.wait_with_output().unwrap();
    assert!(out.status.success(), "{}", text(&out.stderr));
    assert_eq!(images(&out.stdout).iter().map(|d| png_size(&d.png)).collect::<Vec<_>>(), [(30, 20)]);
}

/// A `width` × `height` PNG made by the test.
#[cfg(feature = "image")]
fn picture(test: &str, name: &str, width: u32, height: u32) -> PathBuf {
    let path = tmp(test, name);
    let rgba: Vec<u8> = (0..width * height).flat_map(|i| [(i % 251) as u8, 128, 64, 255]).collect();
    image::save_buffer(&path, &rgba, width, height, image::ExtendedColorType::Rgba8).unwrap();
    path
}

/// An image is one page, shown no larger than it is, and a PDF's options don't apply to it.
#[cfg(feature = "image")]
#[test]
fn an_image_is_one_page_its_own_size_and_takes_no_pdf_options() {
    let png = picture("image", "photo.png", 300, 200);
    let out = view(&png, &["--protocol", "kitty"]);
    assert!(out.status.success(), "{}", text(&out.stderr));
    let drawn = images(&out.stdout);
    assert_eq!(drawn.len(), 1);
    // Its own size, 30 × 10 cells of 10 × 20 px, in an 800 × 460 px window it would fill the
    // height of.
    assert_eq!(png_size(&drawn[0].png), (300, 200));
    assert!(drawn[0].keys.contains(",c=30,r=10,"), "{}", drawn[0].keys);
    // As wide as the window when asked.
    let wide = images(&view(&png, &["--protocol", "kitty", "--per-row", "width"]).stdout);
    assert_eq!(png_size(&wide[0].png), (800, 533));
    let out = view(&png, &["--protocol", "kitty", "--password", "pw"]);
    assert_eq!(text(&out.stderr), format!("clv: --password doesn't apply to {}, which image opens\n", png.display()));
    assert!(!out.status.success() && out.stdout.is_empty());
    let out = view(&png, &["--protocol", "kitty", "--pages", "2"]);
    assert_eq!(text(&out.stderr), "clv: no page 2 (photo.png has 1 page)\n");
}

/// Several files are a list, each by its first page, its name under it. `--pages` picks the
/// list's tiles, in its order.
#[cfg(feature = "image")]
#[test]
fn several_files_are_a_list_each_by_its_first_page() {
    let pdf = fixture("several", 2);
    let png = picture("several", "photo.png", 300, 200);
    let both = |args: &[&str]| view(&pdf, &[&[png.to_str().unwrap(), "--protocol", "kitty"], args].concat());
    let out = both(&[]);
    assert!(out.status.success(), "{}", text(&out.stderr));
    let sizes: Vec<(u32, u32)> = images(&out.stdout).iter().map(|d| png_size(&d.png)).collect();
    assert_eq!(sizes.len(), 2, "{sizes:?}");
    assert_eq!(sizes[1], (300, 200));
    assert_eq!(names(&out.stdout), [" many.pdf photo.png"]);
    // The second tile, not each file's second page: the picture, which has no second page.
    let second = both(&["--pages", "2"]);
    assert!(second.status.success() && second.stderr.is_empty(), "{}", text(&second.stderr));
    assert_eq!(names(&second.stdout), [" photo.png"]);
    // In the order given, each still by its first page.
    let turned = both(&["--pages", "2,1"]);
    let turned_sizes: Vec<(u32, u32)> = images(&turned.stdout).iter().map(|d| png_size(&d.png)).collect();
    assert_eq!((turned_sizes, names(&turned.stdout)), (vec![sizes[1], sizes[0]], vec![" photo.png many.pdf".to_string()]));
    let past = both(&["--pages", "3"]);
    assert!(!past.status.success() && past.stdout.is_empty());
    assert_eq!(text(&past.stderr), "clv: no tile 3 (the list has 2 tiles)\n");
    // An option one of the files takes is no error; one none takes is.
    assert!(both(&["--password", "pw"]).status.success());
    let png2 = picture("several", "other.png", 30, 20);
    let out = view(&png, &[png2.to_str().unwrap(), "--protocol", "kitty", "--password", "pw"]);
    assert_eq!(text(&out.stderr), "clv: --password doesn't apply to any of these files, which image opens\n");
}

#[test]
fn a_file_that_cant_be_shown_among_others_is_left_out() {
    let pdf = fixture("left-out", 2);
    let notes = tmp("left-out", "notes.txt");
    std::fs::write(&notes, "plain text\n").unwrap();
    let out = view(&pdf, &[notes.to_str().unwrap(), "--protocol", "kitty"]);
    assert_eq!((images(&out.stdout).len(), names(&out.stdout)), (1, vec![" many.pdf".to_string()]));
    let why = format!("{}: not a file clv shows", notes.display());
    let stderr = text(&out.stderr);
    assert!(stderr.starts_with(&format!("warning: {why}")), "{stderr}");
    assert!(stderr.ends_with("clv: 1 file left out: notes.txt\n"), "{stderr}");
    assert!(!out.status.success());
    let out = view(&notes, &[notes.to_str().unwrap(), "--protocol", "kitty"]);
    assert!(!out.status.success() && out.stdout.is_empty());
    assert!(text(&out.stderr).ends_with("clv: none of the 2 files can be shown\n"), "{}", text(&out.stderr));
    // A tile past the end of a list made shorter by a file left out: why it is shorter is said.
    let out = view(&pdf, &[notes.to_str().unwrap(), "--protocol", "kitty", "--pages", "2"]);
    assert!(!out.status.success() && out.stdout.is_empty());
    let stderr = text(&out.stderr);
    assert!(stderr.starts_with(&format!("warning: {why}")) && stderr.lines().count() == 2, "{stderr}");
    assert!(stderr.ends_with("\nclv: no tile 2 (the list has 1 tile)\n"), "{stderr}");
}

/// A directory shows each file a backend opens by its first page, and each subdirectory as a
/// folder, in the order of their names (numbers by value); hidden files and files nothing shows
/// are passed over. With `-r`, the subdirectories' files instead of folders.
#[cfg(feature = "image")]
#[test]
fn a_directory_shows_each_of_its_files_by_its_first_page_and_its_subdirectories_as_folders() {
    let dir = tmp("gallery", "photos");
    std::fs::create_dir_all(dir.join("2024")).unwrap();
    std::fs::write(dir.join("a10.pdf"), crate::fixtures::letter("Gallery", 3)).unwrap();
    picture("gallery", "photos/a9.png", 300, 200);
    picture("gallery", "photos/.hidden.png", 50, 50);
    picture("gallery", "photos/2024/b.png", 30, 20);
    std::fs::write(dir.join("notes.txt"), "plain text\n").unwrap();
    let shown = |args: &[&str]| {
        let out = view(&dir, &[&["--protocol", "kitty"], args].concat());
        assert!(out.status.success() && out.stderr.is_empty(), "{args:?}: {}", text(&out.stderr));
        (images(&out.stdout).iter().map(|d| png_size(&d.png)).collect::<Vec<_>>(), names(&out.stdout))
    };
    // 2024 before a9: a folder, wider than tall, then the picture, then the PDF's first page.
    // Three to a row in 80 columns: square places 250 × 240 px, which the picture fits into.
    let (sizes, labels) = shown(&[]);
    assert_eq!((sizes.len(), sizes[1], labels), (3, (250, 167), vec![" 2024/ a9.png a10.pdf".to_string()]), "{sizes:?}");
    assert!(sizes[0].0 > sizes[0].1, "a folder: {:?}", sizes[0]);
    let (w, h) = sizes[2];
    assert!((w as f32 / h as f32 - 612.0 / 792.0).abs() < 0.01, "the PDF's first page only: {w}×{h}");
    // Recursing, a subdirectory's files come at its place among the names.
    for recurse in ["-r", "--recurse"] {
        let (sizes, labels) = shown(&[recurse]);
        assert_eq!((sizes.len(), sizes[0], sizes[1]), (3, (30, 20), (250, 167)), "{recurse}");
        assert_eq!(labels, [" 2024/b.png a9.png a10.pdf"]);
    }
    // A row of tiles at a time, names under each row.
    let out = view(&dir, &["--protocol", "kitty", "-r", "--per-row", "2"]);
    assert_eq!(names(&out.stdout), [" 2024/b.png a9.png", " a10.pdf"]);
    // Among others, a directory is a folder, and an option that a file inside it may take is no
    // error.
    let out = view(&dir, &[dir.join("2024").to_str().unwrap(), "--protocol", "kitty", "--password", "pw"]);
    assert!(out.status.success(), "{}", text(&out.stderr));
    assert_eq!(names(&out.stdout), [" photos/ 2024/"]);
}

/// A directory with nothing to show says so; a file in it that should show and doesn't is left
/// out, as one given by name is.
#[cfg(feature = "image")]
#[test]
fn a_directory_with_nothing_to_show_says_so() {
    let dir = tmp("empty-gallery", "photos");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("notes.txt"), "plain text\n").unwrap();
    let out = view(&dir, &["--protocol", "kitty"]);
    assert!(!out.status.success() && out.stdout.is_empty());
    assert_eq!(text(&out.stderr), format!("clv: {}: nothing in it that clv shows\n", dir.display()));
    // A subdirectory is a folder to show, and recursing, its files.
    std::fs::create_dir_all(dir.join("sub")).unwrap();
    std::fs::write(dir.join("sub/one.pdf"), crate::fixtures::letter("Below", 1)).unwrap();
    assert_eq!(names(&view(&dir, &["--protocol", "kitty"]).stdout), [" sub/"]);
    assert_eq!(images(&view(&dir, &["--protocol", "kitty", "-r"]).stdout).len(), 1);
    // A picture whose header is broken: opened by the image backend, which then fails.
    std::fs::write(dir.join("broken.png"), b"\x89PNG\r\n\x1a\nno chunks").unwrap();
    let out = view(&dir, &["--protocol", "kitty", "-r"]);
    assert_eq!(images(&out.stdout).len(), 1);
    let stderr = text(&out.stderr);
    assert!(stderr.starts_with(&format!("warning: {}: ", dir.join("broken.png").display())), "{stderr}");
    assert!(stderr.ends_with("clv: 1 file left out: broken.png\n") && !out.status.success(), "{stderr}");
}

fn shows() -> String {
    let backends = [
        ("SVG drawings", cfg!(feature = "svg")),
        ("Mermaid diagrams", cfg!(feature = "mermaid")),
        ("tar archives and compressed files", cfg!(feature = "tar")),
        ("documents", cfg!(feature = "word")),
        ("Org documents", cfg!(feature = "org")),
        ("spreadsheets", cfg!(feature = "sheet")),
        ("ebooks", cfg!(feature = "epub")),
        ("PDFs", true),
        ("images", cfg!(feature = "image")),
        ("zip archives", cfg!(feature = "zip")),
    ];
    let shown: Vec<&str> = backends.into_iter().filter_map(|(what, built)| built.then_some(what)).collect();
    match shown.split_last() {
        Some((last, [])) => (*last).to_string(),
        Some((last, rest)) => format!("{} and {last}", rest.join(", ")),
        None => String::new(),
    }
}

/// Errors are said once, under the command's name.
#[test]
fn errors_name_the_command() {
    let out = crate::bin::clv().arg("--per-row").arg("2").output().unwrap();
    assert_eq!(text(&out.stderr), "clv: missing file\n");
    let out = crate::bin::clv().arg("--frobnicate").output().unwrap();
    assert_eq!(text(&out.stderr), "clv: unknown option --frobnicate\n");
}

/// ASCII through a pipe is plain text, for whatever reads it: no escape codes and nothing past
/// ASCII, whatever the environment says of colours, the same width all the way down, and a white
/// page all ink, as nothing says the background is light.
#[test]
fn ascii_through_a_pipe_is_plain_text() {
    let pdf = fixture("ascii", 1);
    let out = view_with(&pdf, &["--protocol", "ascii"], &[("COLORTERM", "truecolor"), ("TERM", "xterm-256color")]);
    assert!(out.status.success() && out.stderr.is_empty(), "{}", text(&out.stderr));
    assert!(out.stdout.is_ascii() && !out.stdout.contains(&0x1b), "{}", text(&out.stdout));
    let page = text(&out.stdout);
    let lines: Vec<&str> = page.lines().collect();
    assert!(lines.len() > 1 && lines.iter().all(|line| line.len() == lines[0].len()), "{page}");
    assert!(page.matches('@').count() > page.len() / 2, "{page}");
    // The text on the page is there, as less ink.
    assert!(lines.iter().any(|line| line.contains(|c| !matches!(c, '@' | ' '))), "{page}");
}

/// Blocks keep the colours the environment says, for a terminal that shows them later, and shade
/// where it says there are none. No graphics either way, and the colours go back at each line's end.
#[test]
fn blocks_take_their_colours_from_the_environment() {
    let pdf = fixture("blocks", 1);
    const WHITE_RGB: &str = "\x1b[48;2;255;255;255m";
    const WHITE_256: &str = "\x1b[48;5;231m";
    let page = |env: &[(&str, &str)]| {
        let out = view_with(&pdf, &["--protocol", "blocks"], env);
        assert!(out.status.success() && out.stderr.is_empty(), "{env:?}: {}", text(&out.stderr));
        let page = text(&out.stdout);
        assert!(!page.contains("\x1b_G"), "{env:?}: {page}");
        page
    };
    let rgb = page(&[("COLORTERM", "truecolor")]);
    assert!(rgb.contains(WHITE_RGB) && !rgb.contains(WHITE_256), "{rgb}");
    assert!(rgb.lines().all(|line| line.ends_with("\x1b[0m")), "{rgb}");
    let palette = page(&[("TERM", "xterm-256color")]);
    assert!(palette.contains(WHITE_256) && !palette.contains(WHITE_RGB), "{palette}");
    for none in [&[("NO_COLOR", "1"), ("COLORTERM", "truecolor")][..], &[]] {
        let shaded = page(none);
        assert!(!shaded.contains('\x1b') && shaded.contains('█'), "{none:?}: {shaded}");
    }
    // An empty NO_COLOR isn't one, as its convention has it.
    assert!(page(&[("NO_COLOR", ""), ("COLORTERM", "truecolor")]).contains(WHITE_RGB));
}

/// Pages side by side are text side by side: each line both pages' cells, with the gap between.
#[test]
fn text_cells_print_in_rows_as_zoomed() {
    let pdf = fixture("ascii-rows", 3);
    let out = view(&pdf, &["--protocol", "ascii", "--per-row", "2"]);
    assert!(out.status.success(), "{}", text(&out.stderr));
    let page = text(&out.stdout);
    let lines: Vec<&str> = page.lines().collect();
    // A page's top is blank, so all ink: two runs of it on a row of two.
    assert_eq!(lines[0].split_whitespace().count(), 2, "{page}");
    // The second row's one page, under the first of the row above.
    let rows: Vec<&[&str]> = lines.split(|line| line.is_empty()).collect();
    assert_eq!(rows.len(), 2, "{page}");
    assert_eq!(rows[1][0].split_whitespace().count(), 1, "{page}");
    assert_eq!(rows[1][0].len(), rows[0][0].find(char::is_whitespace).unwrap(), "{page}");
}

/// A list's tiles are text too, each row with its names under it.
#[test]
fn a_lists_tiles_print_as_text_with_their_names() {
    let (a, b) = (tmp("ascii-list", "first.pdf"), tmp("ascii-list", "second.pdf"));
    std::fs::write(&a, crate::fixtures::letter("A", 1)).unwrap();
    std::fs::write(&b, crate::fixtures::letter("B", 1)).unwrap();
    let out = view(&a, &[b.to_str().unwrap(), "--protocol", "ascii"]);
    assert!(out.status.success(), "{}", text(&out.stderr));
    let page = text(&out.stdout);
    let names = page.lines().last().unwrap();
    assert!(names.contains("first.pdf") && names.contains("second.pdf"), "{page}");
    assert!(page.lines().next().unwrap().split_whitespace().count() == 2, "{page}");
}

/// A page far wider than tall, on the fixture's: its size the same length, so its cross-reference
/// table still finds all. Longer on its long side than a US Letter page, so that fitted to a tile as
/// if it were one, it would come out too wide for its place.
pub(crate) fn wide(title: &str) -> Vec<u8> {
    let portrait = crate::fixtures::letter(title, 1);
    let (from, to) = (b"[0 0 612 792]", b"[0 0 999 200]");
    let at = portrait.windows(from.len()).position(|w| w == from).unwrap();
    [&portrait[..at], &to[..], &portrait[at + from.len()..]].concat()
}

/// What isn't reached costs nothing, however broken: a list's PDFs and documents are opened when
/// their tiles are wanted, so `--pages 1` knows nothing of the rest. Printed, a file that turns out
/// broken costs its own tile, and the error names it at the end.
#[test]
fn a_file_that_turns_out_broken_costs_only_its_tile_and_only_once_reached() {
    let dir = tmp("later-broken", "shelf");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("a.pdf"), crate::fixtures::letter("Fine", 1)).unwrap();
    std::fs::write(dir.join("b.pdf"), b"%PDF-1.7 and nothing a PDF has").unwrap();
    std::fs::write(dir.join("c.pdf"), crate::fixtures::letter("Fine too", 1)).unwrap();
    let out = view(&dir, &["--protocol", "kitty", "--pages", "1"]);
    assert!(out.status.success() && out.stderr.is_empty(), "{}", text(&out.stderr));
    assert_eq!(images(&out.stdout).len(), 1);
    let out = view(&dir, &["--protocol", "kitty"]);
    assert_eq!(images(&out.stdout).len(), 2, "the others are shown");
    assert_eq!(names(&out.stdout), [" a.pdf b.pdf c.pdf"], "its place is kept");
    assert!(!out.status.success() && text(&out.stderr).contains("b.pdf: "), "{}", text(&out.stderr));
    // Given by name, the same: and all of them broken is a list of them, each saying why.
    let (a, b) = (dir.join("a.pdf"), dir.join("b.pdf"));
    let out = view(&b, &[b.to_str().unwrap(), "--protocol", "kitty"]);
    assert!(!out.status.success() && images(&out.stdout).is_empty());
    assert!(text(&out.stderr).contains("b.pdf: "), "{}", text(&out.stderr));
    let out = view(&a, &[b.to_str().unwrap(), "--protocol", "kitty", "--pages", "1"]);
    assert!(out.status.success() && out.stderr.is_empty(), "{}", text(&out.stderr));
    // Alone, a file is opened at once, and its error is the error.
    let out = view(&b, &["--protocol", "kitty"]);
    assert!(!out.status.success() && text(&out.stderr).starts_with(&format!("clv: {}: ", b.display())), "{}", text(&out.stderr));
}

/// A file opened on demand is printed at its true size, not the stand-in it had until then.
#[test]
fn a_tile_opened_on_demand_is_printed_at_its_true_size() {
    let dir = tmp("later-size", "shelf");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("wide.pdf"), wide("Wide")).unwrap();
    std::fs::write(dir.join("tall.pdf"), crate::fixtures::letter("Tall", 1)).unwrap();
    let out = view(&dir, &["--protocol", "kitty"]);
    let sizes: Vec<(u32, u32)> = images(&out.stdout).iter().map(|d| png_size(&d.png)).collect();
    assert_eq!(names(&out.stdout), [" tall.pdf wide.pdf"]);
    let ([tall, wide], true) = (<[(u32, u32); 2]>::try_from(sizes.clone()).unwrap(), out.status.success()) else { panic!("{sizes:?}") };
    assert!(tall.1 > tall.0 && wide.0 > wide.1 * 4, "{sizes:?}");
    // Fitted to its place by its own width: at a US Letter page's scale it would come out wider.
    assert!(wide.0 * 792 < tall.1 * 999 && wide.0 * 10 >= tall.1 * 9, "{sizes:?}");
}

/// Redirected, `/dev/stdin` is a regular file, but on macOS opening its path again goes on from where
/// the first bytes were read to: so among others it is read as it is reached, not come back to by name.
#[cfg(unix)]
#[test]
fn a_file_redirected_in_among_others_is_read_whole() {
    let (input, other) = (tmp("later-stdin", "in.pdf"), tmp("later-stdin", "other.pdf"));
    std::fs::write(&input, crate::fixtures::letter("Redirected", 1)).unwrap();
    std::fs::write(&other, crate::fixtures::letter("Other", 1)).unwrap();
    let out = kept_away()
        .args(["/dev/stdin", other.to_str().unwrap(), "--protocol", "kitty"])
        .stdin(std::fs::File::open(&input).unwrap())
        .output()
        .unwrap();
    assert!(out.status.success() && out.stderr.is_empty(), "{}", text(&out.stderr));
    assert_eq!(images(&out.stdout).len(), 2);
}

/// Only the worker module names the allocator that caps a worker's heap (`cap`): the viewer's own
/// code allocates through it without knowing.
#[test]
fn nothing_but_the_worker_names_cap() {
    crate::fixtures::named_only_in(&["src/source/worker.rs", "tests/it/view.rs"], &[], &["cap::"]);
}

/// What `--worker` is given reaches no terminal as it is: a name no worker has is refused without
/// repeating it, so an escape sequence in it stays out of stderr.
#[test]
fn a_bad_worker_name_is_refused_without_repeating_it() {
    let out = crate::bin::clv().args(["--worker", "bad\x1b]2;injected\x07"]).stdin(Stdio::null()).output().unwrap();
    assert!(!out.status.success());
    assert!(!out.stderr.contains(&b'\x1b'), "{}", String::from_utf8_lossy(&out.stderr));
    assert!(text(&out.stderr).contains("no such worker"), "{}", text(&out.stderr));
}
