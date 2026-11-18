//! The Org backend, through the viewer: which files are Org, by their name or their first line,
//! through a pipe too; what an Org document shows, its TODO keywords in colour and its pictures
//! drawn from beside it; in a list, a zip and a tar; its settings; the limits it is held to, and
//! hostile files, which cost the document and never the viewer; and that nothing else names the
//! library it is parsed with. Its converter's own tests are in `src/source/org.rs`; the pager's
//! (`/`, `s`, `f`) in `tests/it/view_pager.rs`.
#![cfg(all(unix, feature = "org", feature = "image"))]

use std::io::Write as _;
use std::path::Path;
use std::process::{Command, Stdio};

use crate::epub::{BLUE, Mimetype, of, plain, write, zipped};
use crate::view::{images, text, tmp, view};

/// The colour clv writes a TODO keyword in, not done, and done.
const NOT_DONE: [u8; 3] = [0xCF, 0x22, 0x2E];
const DONE: [u8; 3] = [0x1A, 0x7F, 0x37];

/// The pages `clv` printed of `path` with `args`, and what it said.
fn pages(path: &Path, args: &[&str]) -> (Vec<image::RgbImage>, String) {
    let out = view(path, &[&["--protocol", "kitty"], args].concat());
    (images(&out.stdout).iter().map(|page| image::load_from_memory(&page.png).unwrap().to_rgb8()).collect(), text(&out.stderr))
}

/// Whether `page` shows a TODO keyword not done, one done, and the blue picture.
fn shows(page: &image::RgbImage) -> [bool; 3] {
    [NOT_DONE, DONE, BLUE].map(|colour| of(page, colour) > 20)
}

/// Notes with a TODO heading, a DONE one, and a picture beside them, `blue.png`, by `picture`.
fn notes(picture: &str) -> String {
    format!("#+TITLE: Notes\n* TODO Something to do\nSome *bold* text.\n* DONE Something done\n[[{picture}]]\n")
}

/// An Org file is a document, its keywords in colour and the picture it refers to drawn from beside
/// it, `..` included; named `.org` or `.org_archive` in any case.
#[test]
fn an_org_file_is_a_document_its_keywords_in_colour_and_its_picture_drawn() {
    let dir = tmp("org-shown", "notes");
    std::fs::create_dir_all(&dir).unwrap();
    write(dir.join("blue.png"), &plain(80, BLUE));
    for (name, picture) in [("notes.org", "file:blue.png"), ("NOTES.ORG", "./blue.png"), ("old.org_archive", "blue.png")] {
        let path = write(dir.join(name), notes(picture).as_bytes());
        let (pages, said) = pages(&path, &[]);
        let picture = picture != "blue.png";
        assert_eq!((pages.iter().map(shows).collect::<Vec<_>>(), said.as_str()), (vec![[true, true, picture]], ""), "{name}");
    }
    let below = dir.join("below");
    std::fs::create_dir_all(&below).unwrap();
    let path = write(below.join("notes.org"), notes("../blue.png").as_bytes());
    assert_eq!(pages(&path, &[]).0.iter().map(shows).collect::<Vec<_>>(), [[true, true, true]], "from a directory above");
}

/// A file whose first line holds an Emacs mode line naming Org is Org, whatever its name: a `.txt`,
/// a file with no extension, a `.md` (which Markdown would show without the keywords' colours), and
/// a file in UTF-16; and so through a pipe, where there is nothing beside it to draw.
#[test]
fn a_mode_line_says_org_whatever_the_name_and_through_a_pipe() {
    let dir = tmp("org-mode-line", "dir");
    std::fs::create_dir_all(&dir).unwrap();
    write(dir.join("blue.png"), &plain(80, BLUE));
    let org = format!("# -*- mode: org; coding: utf-8 -*-\n{}", notes("file:blue.png"));
    let utf16: Vec<u8> = b"\xFF\xFE".iter().copied().chain(org.encode_utf16().flat_map(u16::to_le_bytes)).collect();
    for (name, bytes) in
        [("notes.txt", org.as_bytes().to_vec()), ("notes", org.as_bytes().to_vec()), ("notes.md", org.as_bytes().to_vec()), ("wide.txt", utf16)]
    {
        let path = write(dir.join(name), &bytes);
        let (pages, said) = pages(&path, &[]);
        assert_eq!((pages.iter().map(shows).collect::<Vec<_>>(), said.as_str()), (vec![[true, true, true]], ""), "{name}");
    }
    let markdown = write(dir.join("plain.md"), notes("blue.png").as_bytes());
    assert_eq!(pages(&markdown, &[]).0.iter().map(shows).collect::<Vec<_>>(), [[false, false, false]], "Markdown without a mode line");

    let mut child = Command::new(env!("CARGO_BIN_EXE_clv"))
        .args(["/dev/stdin", "--protocol", "kitty"])
        .env("TERM", "dumb")
        .env_remove("TMUX")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(org.as_bytes()).unwrap();
    let out = child.wait_with_output().unwrap();
    let printed: Vec<image::RgbImage> = images(&out.stdout).iter().map(|page| image::load_from_memory(&page.png).unwrap().to_rgb8()).collect();
    assert_eq!(printed.iter().map(shows).collect::<Vec<_>>(), [[true, true, false]], "through a pipe: {}", text(&out.stderr));
}

/// Org files in a list, each a tile; in a zip and in a tar, their pictures read from the archive.
#[test]
fn org_files_in_a_list_a_zip_and_a_tar() {
    let dir = tmp("org-held", "list");
    std::fs::create_dir_all(&dir).unwrap();
    let one = write(dir.join("one.org"), b"* TODO One\n");
    let two = write(dir.join("two.org"), b"* DONE Two\n");
    let out = view(&one, &[two.to_str().unwrap(), "--protocol", "kitty"]);
    let tiles: Vec<image::RgbImage> = images(&out.stdout).iter().map(|page| image::load_from_memory(&page.png).unwrap().to_rgb8()).collect();
    assert_eq!(tiles.iter().map(shows).collect::<Vec<_>>(), [[true, false, false], [false, true, false]], "{}", text(&out.stderr));

    let files = vec![("docs/notes.org".to_owned(), notes("file:pics/blue.png").into_bytes()), ("docs/pics/blue.png".to_owned(), plain(80, BLUE))];
    let zip = write(tmp("org-held", "notes.zip"), &zipped(&files, Mimetype::Absent));
    let (tiles, said) = pages(&zip, &["-r"]);
    assert_eq!(tiles.iter().map(shows).filter(|shown| shown[0]).collect::<Vec<_>>(), [[true, true, true]], "in a zip: {said}");

    let tar = tar_of(&files);
    let tar = write(tmp("org-held", "notes.tar"), &tar);
    let (tiles, said) = pages(&tar, &["-r"]);
    assert_eq!(tiles.iter().map(shows).filter(|shown| shown[0]).collect::<Vec<_>>(), [[true, true, true]], "in a tar: {said}");
}

/// A tar of `files`, by name and contents, written as ustar.
fn tar_of(files: &[(String, Vec<u8>)]) -> Vec<u8> {
    let mut out = Vec::new();
    for (name, bytes) in files {
        let mut header = [0u8; 512];
        header[..name.len()].copy_from_slice(name.as_bytes());
        header[100..108].copy_from_slice(b"0000644\0");
        header[108..116].copy_from_slice(b"0000000\0");
        header[116..124].copy_from_slice(b"0000000\0");
        header[124..136].copy_from_slice(format!("{:011o}\0", bytes.len()).as_bytes());
        header[136..148].copy_from_slice(b"00000000000\0");
        header[156] = b'0';
        header[257..263].copy_from_slice(b"ustar\0");
        header[263..265].copy_from_slice(b"00");
        header[148..156].copy_from_slice(b"        ");
        let sum: u32 = header.iter().map(|b| u32::from(*b)).sum();
        header[148..156].copy_from_slice(format!("{sum:06o}\0 ").as_bytes());
        out.extend_from_slice(&header);
        out.extend_from_slice(bytes);
        out.resize(out.len().div_ceil(512) * 512, 0);
    }
    out.resize(out.len() + 1024, 0);
    out
}

/// What Markdown has, an Org document has: dark paper, a width in characters, and its source
/// blocks drawn as pictures by the backend that draws their language, or shown as code.
#[test]
fn an_org_document_has_the_settings_markdown_has() {
    let path =
        write(tmp("org-settings", "notes.org"), b"* TODO Something\nSome text, long enough to wrap at a narrow width but not at a wide one.\n");
    let (light, _) = pages(&path, &[]);
    let (dark, _) = pages(&path, &["--theme", "dark"]);
    let corner = |page: &image::RgbImage| page.get_pixel(2, 2).0;
    assert!(corner(&light[0]).iter().all(|c| *c > 200) && corner(&dark[0]).iter().all(|c| *c < 80), "{:?} {:?}", corner(&light[0]), corner(&dark[0]));
    let (narrow, _) = pages(&path, &["--width", "20"]);
    let (wide, _) = pages(&path, &["--width", "120"]);
    assert!(narrow[0].width() < wide[0].width(), "{} {}", narrow[0].width(), wide[0].width());
}

/// A source block in a language a backend draws (`#+begin_src mermaid`) is that picture, and with
/// `--fences code` its code.
#[cfg(feature = "mermaid")]
#[test]
fn an_org_documents_mermaid_block_is_drawn_or_shown_as_code() {
    let flow = std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/flow.mmd")).unwrap();
    let path = write(tmp("org-fences", "flow.org"), format!("* A diagram\n#+begin_src mermaid\n{flow}#+end_src\n").as_bytes());
    let red_and_blue = |page: &image::RgbImage| {
        let has = |colour: [u8; 3]| page.pixels().any(|pixel| pixel.0.iter().zip(colour).all(|(a, b)| a.abs_diff(b) < 40));
        has([255, 0, 0]) && has([0, 0, 255])
    };
    let (drawn, said) = pages(&path, &[]);
    assert!(red_and_blue(&drawn[0]), "drawn: {said}");
    let (code, said) = pages(&path, &["--fences", "code"]);
    assert!(!red_and_blue(&code[0]), "as code: {said}");
}

/// The Org source is held to the documents' limits before it is parsed, and the HTML it is written
/// as to HTML's after: a file within Org's own, whose table makes more marks of HTML than HTML may
/// have, is refused, and said to be for its HTML.
#[test]
fn an_org_file_is_held_to_the_documents_limits_and_its_html_to_htmls() {
    let refused = |name: &str, bytes: &[u8]| {
        let path = write(tmp("org-limits", name), bytes);
        let out = view(&path, &["--protocol", "kitty"]);
        assert!(!out.status.success() && out.stdout.is_empty(), "{name}");
        text(&out.stderr)
            .strip_prefix(&format!("clv: {}: ", path.display()))
            .unwrap_or_else(|| panic!("{name}: {}", text(&out.stderr)))
            .trim_end()
            .to_owned()
    };
    assert_eq!(refused("marks.org", "*a".repeat(500_001).as_bytes()), "the document has more than 500000 lines and marks");
    // Twelve lines and marks a row of Org, twenty-two of HTML.
    let table = "|a|b|c|d|e|f|g|h|i|j|\n".repeat(40_000);
    assert_eq!(refused("table.org", table.as_bytes()), "as the HTML it is written as, the document has more than 500000 lines and marks");
}

/// What orgize takes longest on, special blocks nested 10,000 deep, costs the document and never the
/// viewer: the worker is killed past its CPU time, and the viewer says why and ends as it does for
/// any file it can't show; so for blocks nested past what a stack holds.
#[test]
fn a_hostile_org_file_costs_the_document_and_never_the_viewer() {
    let nested: String = (0..10_000)
        .map(|n| format!("#+begin_b{n}\n"))
        .chain(std::iter::once("x\n".to_owned()))
        .chain((0..10_000).rev().map(|n| format!("#+end_b{n}\n")))
        .collect();
    let footnotes = "[fn:: ".repeat(90_000) + "x" + &"]".repeat(90_000) + "\n";
    for (name, org) in [("nested.org", nested), ("footnotes.org", footnotes)] {
        let path = write(tmp("org-hostile", name), org.as_bytes());
        let started = std::time::Instant::now();
        let out = view(&path, &["--protocol", "kitty"]);
        assert!(out.status.code().is_some(), "{name}: it ended by a signal");
        assert!(started.elapsed() < std::time::Duration::from_secs(20), "{name}: {:?}", started.elapsed());
        let said = text(&out.stderr);
        let why = said.strip_prefix(&format!("clv: {}: ", path.display())).map(str::trim_end).unwrap_or(&said);
        assert!(images(&out.stdout).is_empty() && why.starts_with("orgize "), "{name}: {why}");
    }
}

/// `--help` lists Org with its extensions, crediting orgize.
#[test]
fn help_lists_org_and_credits_orgize() {
    let out = Command::new(env!("CARGO_BIN_EXE_clv")).arg("--help").env("TERM", "dumb").output().unwrap();
    assert!(text(&out.stdout).contains("Org documents, by orgize: Org (.org .org_archive)"), "{}", text(&out.stdout));
}

#[test]
fn nothing_but_this_backend_names_orgize() {
    crate::fixtures::named_only_in(&["src/source/org.rs", "tests/it/org.rs"], &[], &["orgize"]);
}
