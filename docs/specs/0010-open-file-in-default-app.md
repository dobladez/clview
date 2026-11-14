# Spec-0010: Open a file in the desktop's default app

- **Status:** Done
- **Date:** 2026-11-14
- **Related:** [ADR-0009](../adr/0009-copy-a-page-through-the-terminals-clipboard-protocol-else-the-systems.md)
  (`c`'s system clipboard, used only on this machine, the pattern this follows);
  [ADR-0033](../adr/0033-backends-share-a-setting-by-declaring-its-name.md) and
  [Layers](../architecture.md#layers) (the viewer knows no format, nor any backend);
  [Copying a page](../usage.md#copying-a-page), [On this machine or over SSH](../usage.md#on-this-machine-or-over-ssh),
  [Keys](../usage.md#keys), [Copying to the clipboard](../architecture.md#copying-to-the-clipboard-cliprs);
  the backlog's [`O`](../backlog.md#o-open-a-file-in-an-app-of-the-readers-choosing),
  [an archive's file](../backlog.md#open-an-archives-file-in-another-app) and
  [more openers](../backlog.md#more-openers-wsl-among-them), which come after it.
  It delivers none of README's Plans.

## Summary

`o` opens what the reader is looking at in the app the desktop opens it with: in a file's pages,
that file; in a list, the tile chosen, a file or a directory's folder (shown in the file manager).
It hands a document over to a real editor or viewer: a PDF to fill in, a Word document to edit, a
photo to retouch. `clv` runs the platform's opener (`open` on macOS, `xdg-open` on Linux and the
BSDs' desktops) with the file's real, absolute path, without a shell and without waiting; the
status line says `opening report.pdf…`, then that it did, or why not. Each opener is one entry
behind a small trait, in a list tried in order, as `source::BACKENDS` lists backends: another
desktop or platform is another entry and its test, and nothing else changes.

What can't be opened is said instead: a file only inside an archive, or from a pipe or stdin, is
no file of its own; over SSH the app would open on the far machine, not the reader's; with no
desktop there is nothing to open it in; and a file whose real name doesn't say the format `clv`
showed it as is not opened, since the desktop picks the app by the name (a `photo.command` that
starts as a PNG does, or a `photo.png` linked to it, would run as a script).

## Requirements

- **`o` in a file's pages** opens that file, whatever page or search is on screen, given on the
  command line or gone into from a list, and still after the document is laid out again. **In a
  list**, the tile chosen: a file, even one `clv` couldn't show (its tile kept, opening or broken),
  since the app may read it; a directory's folder.
- **The real path.** The path is resolved (`std::fs::canonicalize`: symbolic links followed, made
  absolute), and what follows is checked on what it resolves to and launched as that, a single
  argument with no shell: a name starting with `-`, or holding spaces or quotes, reaches the opener
  as it is. Resolved under `/dev` or `/proc` (`/dev/stdin`, `/dev/fd/0`, even redirected from a
  file), it is refused as stdin is. A file swapped for another between the check and the launch is
  a race no hand-over by path can close: said in the ADR, not guarded against.
- **The name agrees with the format.** A file is opened only if its real extension, in any case, is
  one that files of a format of the backend that opened it go by: `.pdf` for a PDF, `.png`, `.jpg`
  and the rest for a picture, `.md`, `.html` and the rest for a document. Otherwise the status line
  says so, naming what `clv` showed it as in the backend's own words (`Backend::shows`), never a
  format written in the core: `photo.command: its name doesn't say pictures, so it isn't opened`.
- **A file compressed alone** (`report.pdf.gz`), shown through the archives' backend, opens as the
  compressed file it is: that backend's formats name the compressed extensions (`.gz`, `.bz2`,
  `.xz`), and the desktop's archive tool takes it.
- **A folder** opens in the file manager. On macOS every folder is shown in Finder (`open -R`), never
  opened: Finder takes a folder for a package (an app, which `open` would launch) by its extension or
  by a flag in its metadata, so no name tells them apart.
- **Without waiting.** The opener starts with stdin, stdout and stderr null, in a process group of
  its own (so `Ctrl-C` in `clv` doesn't reach it), and on Linux and the BSDs through `setsid`, so it
  has no controlling terminal: an app for the terminal fails rather than taking `clv`'s. Nothing it
  writes reaches the screen, and no pipe ties it to `clv` (an app it starts would inherit one, and
  could die of `SIGPIPE` once `clv` is gone). A thread waits for it, as long as it runs, so it is
  never left a zombie; the keys keep working meanwhile.
- **The status line:** `opening report.pdf…`, then `opened report.pdf`, or `report.pdf: <why>`, the
  why being its exit status in the launcher's words (`xdg-open`'s 2 is the file not found, 3 a tool
  it needs missing, 4 the action failed; `open`'s 1). Past 5 s without an answer, it says the file
  was handed to the opener, still waited for on its thread. At most 4 launches are outstanding: `o`
  past them says one is still under way.
- **Refused, saying why:** a file held in an archive (zip, tar), and an archive's folder; what isn't
  a regular file, or resolves under `/dev` or `/proc`; over SSH, by the `local` that `c` goes by; no
  opener applying here (Linux with neither `DISPLAY` nor `WAYLAND_DISPLAY`), or its program not
  found. A name that isn't UTF-8 is already left out of a list ([Limits](../usage.md#limits)); a
  launch path is never made from a name as the status line shows it.
- **The openers:** a trait, in a list tried in order, the first that applies used: it says whether
  it applies here, the command that opens a path, and its exit statuses in words. Its program is
  found on `PATH`, by name. To begin with, macOS's `open`; on Linux and the BSDs with a desktop,
  `xdg-open`, through `setsid` where it is found.
- **`?`** lists `o` in a file's keys and a list's.
- Nothing when printing (`--no-pager`), where no keys are read.

## Out of scope

- **`O`, choosing the app:** the candidates listed (Launch Services, freedesktop's MIME-apps lists)
  and a picker over the pages. The trait is shaped so that it adds methods
  ([backlog](../backlog.md#o-open-a-file-in-an-app-of-the-readers-choosing)).
- **An archive's file**, unpacked to a temporary file to open: when it could be deleted, with the app
  still reading, is the question ([backlog](../backlog.md#open-an-archives-file-in-another-app)).
- **WSL and more openers** (`wslview`, `gio open`, `kde-open`, `exo-open`, `termux-open`, Haiku's
  `open`): an entry and a test each, later or from a pull request
  ([backlog](../backlog.md#more-openers-wsl-among-them)). `wslview` hands the path on to PowerShell
  or `cmd.exe`, a parser of its own, so it waits for tests on a real WSL. `xdg-open` reaches most
  desktops through their own tools.
- **The command chosen by the reader** (`zathura` for PDFs): the [config file](../backlog.md#a-config-file)'s.
- **Opening at the page the reader is on**: Preview can't be told one; Skim, Zathura and Okular can,
  each its own way, so it comes with `O`, an app at a time.
- **On the reader's machine over SSH**, through the terminal: no terminal offers that for files that
  we know of, so it is refused there.
- **tmux's environment:** inside tmux, the pane's `DISPLAY` and `WAYLAND_DISPLAY` are taken as they
  are, even if stale from an earlier attach; the session isn't asked for fresher ones.
- **A key showing a file in the file manager** (`open -R`), beyond the folders.

## Approach

**Files:**

- `src/view/launch.rs` (new, the core): the `Launcher` trait, `LAUNCHERS`, and the launches under
  way, each waited for on a thread of its own. Named so beside `view/mod.rs`'s `Opener`, which opens a
  list's files. Not a backend, and behind no cargo feature: every launcher is built everywhere and
  decides at run time whether it applies, so each is tested on any machine.

  ```rust
  pub(crate) trait Launcher {
      fn name(&self) -> &'static str;                          // "open", "xdg-open"
      fn applies(&self, here: &Here) -> bool;                  // the OS and the environment, given, not read
      fn command(&self, path: &Path, folder: bool) -> Command; // the path real and absolute
      fn why(&self, status: ExitStatus) -> String;             // its exit status in words
  }
  ```

  `O` would add `apps(path)` and a command for the app picked, with defaults, so no launcher breaks.
- `src/view/pager.rs`: `o` a `Step::Launch(page)`; `launch()` and `launch_on()` as `copy()` and
  `copy_on()` are (a message at once, the outcome polled each time round the loop); `PAGE_HELP`,
  `LIST_HELP`, `VIEWER_KEYS`. What a file's pages were opened from (its `OnDisk`) is kept by `Level`,
  through `Level::new`, `GoingInto` and `go_on_into` (a tile gone into), and `replace` (laid out
  again), each of which builds its `Pages` anew today.
- `src/view/mod.rs`: `Pages` and `Item::Later` carry an `OnDisk { path, named }` for a file of its
  own, filled in by `Opener::file` before its `Input` is consumed; `Terminal.local` reaches the pager
  as it does the copier.
- `src/source.rs`: `Input::on_disk()` (a regular file, `Rest::File`, never one held; `later.rs`
  already tells a path that can be opened again from one that can't); `Format`'s `named`, the
  extensions its files go by, beside `extensions`, which `opens` goes by and which stays as it is;
  `named_as(backend, path)`; `Container::on_disk()`, `None` by default, the directory's path for
  `dir.rs`.
- The backends' `Format` tables: `pdfcraft.rs`, `image.rs`, `svg.rs`, `wordcraft.rs`,
  `gridcraft.rs`, `mermaid.rs`, and `tar.rs` for its files compressed alone. The archives' folders
  are refused, so they need none.
- `tests/it/view_pager.rs`: end to end, `PATH` set to a directory of fake `open`, `xdg-open` and
  `setsid` (each writing its arguments to a file) and `/bin`, so no test reaches a real desktop app;
  `DISPLAY`, `WAYLAND_DISPLAY` and the `SSH_*` variables set or removed by each test, not inherited.
- `docs/usage.md` (Keys; a section "Opening in another app" beside Copying a page),
  `docs/architecture.md` (a section beside Copying to the clipboard), `CHANGELOG.md`, ADR-0034.

**Patterns to follow:**

- `clip.rs`'s system clipboard: `local` decides, the work on a thread, the status line following.
- `source::BACKENDS` and `source::WORKERS`: a list of trait objects, the first that applies used.

**Decided:**

- `std::process`, not the `open` or `opener` crates: no dependency, and the launchers a list of our
  own to extend. ADR-0034 records it, with the name-agrees rule and its `photo.command`, the real
  path and the race it leaves, every macOS folder revealed, and why no pipe.
- No shell; stdin, stdout and stderr null; a process group of its own
  (`CommandExt::process_group(0)`); `setsid` on Linux and the BSDs, started without `-f`, so the
  opener keeps its process and its exit status. Not `setsid()` itself, which takes `unsafe`.
- The status line waits 5 s; the thread as long as the opener runs; 4 launches at most.
- The name agrees with any of the opening backend's formats, not only the one it was read as: a
  `.md` read as HTML still opens. It stops a dangerous mismatch without second-guessing formats.

## Open questions

🔴 blocks it · 🟡 to keep in mind.

- 🟡 **`xdg-open` that blocks** — some of its fallbacks wait for the app to exit; the status line
  stops waiting at 5 s, and the thread reaps it whenever it ends.
- 🟡 **`setsid` missing** — util-linux and BusyBox have it; where neither does (some BSDs), `o` is
  refused, saying so. Revised after a Codex review: in its own process group alone, the opener
  keeps `clv`'s controlling terminal, and an app for the terminal opening `/dev/tty` writes to it
  freely, `TOSTOP` being off as it usually is: only `SIGTTIN` would stop it, not a write.

## Tasks

- [x] **`o` on a file given on the command line** — the launchers (`open`, `xdg-open`) and the
  launches on their threads; `Format.named` in every backend, `Input::on_disk`, `named_as`, the real
  path and its refusals (a mismatched name, a link to one, `/dev/stdin`, a pipe, over SSH, no
  desktop); `o` in a file's pages, `?`, the status line; ADR-0034, usage, architecture and the
  changelog. *Files:* `src/view/launch.rs`, `src/view/mod.rs`, `src/view/pager.rs`, `src/source.rs`,
  `src/source/*.rs`, `tests/it/view_pager.rs`, `docs/`, `CHANGELOG.md`. *Verify:* `cargo test -j 2
  --bin clv launch` (which launcher applies for given environments, their arguments, an exit status
  in words, a launch reaped after the 5 s, the fourth one refused, with `true`, `false` and `sleep`
  as openers), `cargo test -j 2 --bin clv named`, and `cargo test -j 2 --test it view_pager::open`.
- [x] **`o` in a list** — a tile's file, one still opening and a broken one, a folder (revealed on
  macOS), an archive's file and folder refused, a file compressed alone; the path kept going into a
  tile and laid out again. *Files:* `src/view/mod.rs`, `src/view/pager.rs`, `src/source.rs`,
  `src/source/dir.rs`, `src/source/later.rs`, `src/source/tar.rs`, `tests/it/view_pager.rs`, `docs/`.
  *Verify:* `cargo test -j 2 --test it view_pager::open`, and `Manual: in kitty on macOS, clv
  tests/data, press o on a PDF's tile: Preview opens it; on a folder: Finder shows it.`

## Done

- [x] The quality gates in [`CONTRIBUTING.md`](../../CONTRIBUTING.md#quality-gates) pass.
- [x] Tests added, each seen failing without the change: name them (in the
  [notes](0010-open-file-in-default-app-impl-20261114_1858.md)).
- [x] Manual: in kitty on macOS, `o` on a file's pages, a list's file, a folder, an archive's file
  (refused), a `.png` linked to a `.command` (refused), and over `ssh localhost` (refused).
- [x] The docs say what shipped ([When a spec is Done](README.md#when-a-spec-is-done)).

## Changelog

- 2026-11-14 — Ready (spec-new).
- 2026-11-14 — Revised after a Codex review: the real path checked, every macOS folder revealed,
  WSL left for later, no pipes, `setsid`, the opener reaped however long it runs, the path kept
  through a level's life, two tasks.
- 2026-11-14 — Implementing ([notes](0010-open-file-in-default-app-impl-20261114_1858.md)).
- 2026-11-14 — After a Codex review of task 1: `xdg-open` never runs without `setsid`, `o`
  refused instead (the 🟡 on `setsid` missing).
- 2026-11-14 — Done.
