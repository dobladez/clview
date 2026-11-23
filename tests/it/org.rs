//! The Org backend, through the viewer: which files are Org, by their name or their first line,
//! through a pipe too; what an Org document shows, its TODO keywords in colour and its pictures
//! drawn from beside it; in a list, a zip and a tar; its settings; the limits it is held to, and
//! hostile files, which cost the document and never the viewer; and that nothing else names the
//! library it is parsed with. Its converter's own tests are in `src/source/org.rs`; the pager's
//! (`/`, `s`, `f`) in `tests/it/view_pager.rs`.
#![cfg(all(unix, feature = "org", feature = "image"))]

use std::io::Write as _;
use std::path::Path;
use std::process::Stdio;

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
/// a file with no extension, a file in UTF-16; and so through a pipe, where there is nothing beside
/// it to draw. But a `.md` with that line is Markdown, which the documents' backend, asked first,
/// is sure of (the owner's choice, 2026-11-18): without the keywords' colours.
#[test]
fn a_mode_line_says_org_whatever_the_name_and_through_a_pipe() {
    let dir = tmp("org-mode-line", "dir");
    std::fs::create_dir_all(&dir).unwrap();
    write(dir.join("blue.png"), &plain(80, BLUE));
    let org = format!("# -*- mode: org; coding: utf-8 -*-\n{}", notes("file:blue.png"));
    let utf16: Vec<u8> = b"\xFF\xFE".iter().copied().chain(org.encode_utf16().flat_map(u16::to_le_bytes)).collect();
    for (name, bytes) in [("notes.txt", org.as_bytes().to_vec()), ("notes", org.as_bytes().to_vec()), ("wide.txt", utf16)] {
        let path = write(dir.join(name), &bytes);
        let (pages, said) = pages(&path, &[]);
        assert_eq!((pages.iter().map(shows).collect::<Vec<_>>(), said.as_str()), (vec![[true, true, true]], ""), "{name}");
    }
    for (name, text) in [("plain.md", notes("blue.png")), ("said.md", org.clone())] {
        let markdown = write(dir.join(name), text.as_bytes());
        assert_eq!(pages(&markdown, &[]).0.iter().map(shows).collect::<Vec<_>>(), [[false, false, false]], "{name}: Markdown, its Org as text");
    }

    let mut child = crate::bin::clv()
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

/// Through a pipe, an Org file is read to a byte past the documents' limit and no further: the
/// viewer refuses it and stops reading, so what writes to the pipe finds it closed.
#[test]
fn an_org_file_through_a_pipe_is_never_read_past_the_limit() {
    let mut child = crate::bin::clv()
        .args(["/dev/stdin", "--protocol", "kitty"])
        .env("TERM", "dumb")
        .env_remove("TMUX")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    let writer = std::thread::spawn(move || -> std::io::Result<()> {
        stdin.write_all(b"-*- org -*-\n")?;
        let chunk = vec![b'a'; 1 << 20];
        for _ in 0..200 {
            stdin.write_all(&chunk)?;
        }
        Ok(())
    });
    let out = child.wait_with_output().unwrap();
    let wrote = writer.join().unwrap();
    assert!(!out.status.success() && text(&out.stderr).contains("the document is more than 64 MiB"), "{}", text(&out.stderr));
    assert!(wrote.is_err_and(|e| e.kind() == std::io::ErrorKind::BrokenPipe), "the pipe was read to its end");
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
    let out = crate::bin::clv().arg("--help").env("TERM", "dumb").output().unwrap();
    assert!(text(&out.stdout).contains("Org documents, by orgize: Org (.org .org_archive)"), "{}", text(&out.stdout));
}

#[test]
fn nothing_but_this_backend_names_orgize() {
    crate::fixtures::named_only_in(&["src/source/org.rs", "tests/it/org.rs"], &[], &["orgize"]);
}

/// `f` and a label follows a link, here to a page past the first; the page it lands on.
fn followed(view: &mut crate::view_pager::Session, links: usize, label: &str) -> usize {
    use crate::view_pager::{letters, place_now};
    let before = view.text().len();
    view.press("f");
    view.wait_until("the labels", |t| letters(&t[before..]).len() == links);
    let before = view.text().len();
    view.press(label);
    view.wait_until("the link followed", |t| t.len() > before && place_now(&t[before..]).is_some_and(|(page, _)| page > 1));
    std::thread::sleep(std::time::Duration::from_millis(200));
    place_now(&view.text()).unwrap().0
}

/// Back with Backspace, to the first page.
fn back(view: &mut crate::view_pager::Session) {
    let before = view.text().len();
    view.press("\x7f");
    view.wait_until("back", |t| t.len() > before && crate::view_pager::place_now(&t[before..]).is_some_and(|(page, _)| page == 1));
}

/// Filler, a page of it and more at the window's size.
fn filler() -> String {
    "Filler, with words enough in it to fill a line of a page.\n\n".repeat(60)
}

/// The links of an Org document lead where Org has them: `[[*Heading]]` to the heading, without its
/// keyword, priority, tags and cookies, a heading whose title wraps a line in the link too;
/// `[[#custom-id]]` and `[[id:X]]` to the heading that has it; `[[target]]` to a target before a
/// heading of the same title; a footnote's reference to its note, each on a later page, in order.
#[test]
fn links_in_org_go_to_headings_targets_ids_and_notes() {
    if !crate::view_pager::pty_works() {
        return;
    }
    let filler = filler();
    let org = format!(
        "[[*First Heading]] [[#custom]] [[id:abc-123]] [[*Second\nheading wrapped]] [[clash]] [[*clash]] Note[fn:1]\n\n{filler}\
         * TODO [#B] First Heading [1/2] :tag:\n{filler}\
         * Second heading wrapped\n:PROPERTIES:\n:CUSTOM_ID: custom\n:ID: abc-123\n:END:\n{filler}\
         * clash\n{filler}A <<clash>> target.\n{filler}\
         [fn:1] The note.\n"
    );
    let path = write(tmp("org-links", "notes.org"), org.as_bytes());
    let mut view = crate::view_pager::Session::start(&path, &["--protocol", "kitty", "--pager"], 100, 60);
    view.wait_until("page 1", |t| crate::view_pager::place_now(t).is_some_and(|(page, _)| page == 1));
    let mut pages = Vec::new();
    for label in ["s", "a", "d", "f", "j", "k", "l"] {
        pages.push(followed(&mut view, 7, label));
        back(&mut view);
    }
    let [first, custom, id, wrapped, target, heading, note] = pages[..] else { unreachable!() };
    assert!(first > 1 && custom > first && id == custom && wrapped == custom, "{pages:?}");
    assert!(heading > wrapped && target > heading && note > target, "the target, not the heading of its name, then the note: {pages:?}");
    view.press("q");
    assert!(view.finish().success());
}

/// A link to another Org file goes to its heading, by the title written after `::*`, and a
/// Markdown file's link to it by GitHub's slug of the heading; Backspace comes back.
#[test]
fn links_to_an_org_file_go_to_its_heading_from_org_and_from_markdown() {
    if !crate::view_pager::pty_works() {
        return;
    }
    let filler = filler();
    let dir = tmp("org-links-other", "dir");
    std::fs::create_dir_all(&dir).unwrap();
    write(dir.join("other.org"), format!("* Start\n{filler}* Some Heading\nHere.\n").as_bytes());
    for (name, text) in [("from.org", "See [[file:other.org::*Some Heading][there]].\n"), ("from.md", "See [there](other.org#some-heading).\n")] {
        let path = write(dir.join(name), text.as_bytes());
        let mut view = crate::view_pager::Session::start(&path, &["--protocol", "kitty", "--pager"], 100, 60);
        view.wait_until("page 1", |t| crate::view_pager::drawn(t, "page 1/1"));
        let before = view.text().len();
        view.press("f");
        view.wait_until("the label", |t| crate::view_pager::letters(&t[before..]).len() == 1);
        view.press("s");
        view.wait_until("the other file at its heading", |t| {
            crate::view_pager::last_bar(t).contains("other.org") && crate::view_pager::place_now(t).is_some_and(|(page, _)| page > 1)
        });
        view.press("\x7f");
        view.wait_until("back", |t| crate::view_pager::last_bar(t).contains(name));
        view.press("q");
        assert!(view.finish().success(), "{name}");
    }
}

/// Across files, `::*Heading` goes to the heading and `::target` to the target of the same name,
/// each where it is in the other file.
#[test]
fn links_to_an_org_files_heading_go_to_the_heading_never_a_target_of_its_name() {
    if !crate::view_pager::pty_works() {
        return;
    }
    let filler = filler();
    let dir = tmp("org-links-clash", "dir");
    std::fs::create_dir_all(&dir).unwrap();
    write(dir.join("other.org"), format!("* Start\n{filler}A <<clash>> target.\n{filler}* clash\nThe heading.\n").as_bytes());
    let path = write(dir.join("from.org"), b"[[file:other.org::*clash][the heading]] and [[file:other.org::clash][the target]]\n");
    let mut view = crate::view_pager::Session::start(&path, &["--protocol", "kitty", "--pager"], 100, 60);
    view.wait_until("page 1", |t| crate::view_pager::drawn(t, "page 1/1"));
    let mut pages = Vec::new();
    for label in ["s", "a"] {
        let before = view.text().len();
        view.press("f");
        view.wait_until("the labels", |t| crate::view_pager::letters(&t[before..]).len() == 2);
        let before = view.text().len();
        view.press(label);
        view.wait_until("the other file", |t| {
            t.len() > before
                && crate::view_pager::last_bar(&t[before..]).contains("other.org")
                && crate::view_pager::place_now(&t[before..]).is_some_and(|(page, _)| page > 1)
        });
        std::thread::sleep(std::time::Duration::from_millis(200));
        pages.push(crate::view_pager::place_now(&view.text()).unwrap().0);
        let before = view.text().len();
        view.press("\x7f");
        view.wait_until("back", |t| t.len() > before && crate::view_pager::last_bar(&t[before..]).contains("from.org"));
    }
    assert!(pages[0] > pages[1], "the heading after the target: {pages:?}");
    view.press("q");
    assert!(view.finish().success());
}

/// A link that would run something (`elisp:`, `shell:`) is never followed, the status line saying
/// what it is; an `id:` this file hasn't is said not to be in it.
#[test]
fn links_that_run_something_or_lead_nowhere_say_so() {
    if !crate::view_pager::pty_works() {
        return;
    }
    let path = write(tmp("org-links-said", "notes.org"), b"[[elisp:(beep)][run]] [[id:elsewhere][gone]]\n");
    let mut view = crate::view_pager::Session::start(&path, &["--protocol", "kitty", "--pager"], 100, 60);
    view.wait_until("page 1", |t| crate::view_pager::drawn(t, "page 1/1"));
    for (label, said) in [("s", "an elisp: link, which isn't followed"), ("a", "no #id:elsewhere in this document")] {
        let before = view.text().len();
        view.press("f");
        view.wait_until("the labels", |t| crate::view_pager::letters(&t[before..]).len() == 2);
        view.press(label);
        view.wait_until(said, |t| t.get(before..).is_some_and(|t| t.contains(said)));
    }
    view.press("q");
    assert!(view.finish().success());
}
