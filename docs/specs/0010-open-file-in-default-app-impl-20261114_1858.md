# Spec-0010: implementation notes (2026-11-14)

Notes made while implementing [Spec-0010](0010-open-file-in-default-app.md): decisions, changes from its plan,
surprises, and what a later reader would want.

## Task 1: `o` on a file given on the command line

- **`Format::names`, not `named`.** The spec calls the new field `named`, but `Format` already has a
  method `named(path)` (whether a path ends in one of its `extensions`, which `opens` goes by), so
  the field is `names`, set by `Format::of` to the `extensions` and by `Format::named_by` to
  others. A test checks every format of every backend in the build has some.
- **`OnDisk` and where the refusals are worded.** `Input::on_disk()` returns the real path or why
  there is none, and `source::named_as(backend, path)` whether the name agrees, or why not in the
  backend's words; the core keeps both as `view::OnDisk { path, named }` inside a `Place`
  (`Result<OnDisk, String>`) on `Pages`, `Item::Later` and `Level`. A name refused is said with the
  real name (`report.command: …`), so a link's reader sees why; the rest with the name shown.
- **`/dev` and `/proc` checked on the path as given too, not only resolved.** On Linux,
  `/dev/stdin` redirected from a file resolves through `/proc/self/fd/0` to the file itself, so the
  resolved path alone would let it through. A link of the reader's own to `/dev/stdin` still
  resolves to the file there: it opens as the file it is (in the ADR).
- **`setsid` without a process group of its own.** The brief said `process_group(0)` and, on Linux
  and the BSDs, `setsid` when found. `setsid(1)` forks when it starts as a process group's leader
  (its manual), its parent exiting without the opener's status, so with `setsid` the child isn't
  put in a group first: `setsid` makes a session and a group of its own. `open`, and `xdg-open`
  without `setsid`, get `process_group(0)`. (The second changed after the Codex review below.)
- **The trait** takes `here` in `command` and returns a `Result` (its program found on `PATH`, or
  why not), beside the spec's `name`, `applies`, `why`.
- **The thread before the opener**: started first, so that an opener started always has one to
  wait for it; the child goes to it over a channel.
- **After 5 s**, a late success isn't said (the reader was told it was handed over), a late failure
  is.
- **Level and replace** carry the `Place` in this task already, since a document laid out again
  makes its `Pages` anew; going into a list's tile does too (`GoingInto`). Their tests are task 2's.
- **The tests' `PATH`.** Every `Session` in `tests/it/view_pager.rs` now runs `clv` with a `PATH`
  of fake openers that do nothing, then `/bin` and `/usr/bin` (set in the shell line, since
  `script` itself is found on the tests' own), and without `DISPLAY` or `WAYLAND_DISPLAY`: no test
  can reach a real desktop app, whatever it presses. The opening tests use their own fakes, which
  note their arguments, and `/bin` only; the one for no opener, an empty directory alone, since on
  a merged-`/usr` Linux `/bin` holds `xdg-open`.
- **`setsid` checked by a test**: `with_setsid_the_opener_leads_no_group_and_without_it_leads_its_own`
  (now `…_and_open_leads_its_own`)
  runs the commands built, with fakes that note their own pid and process group: with `setsid`, it
  starts as no group's leader; `xdg-open` without it, and `open`, lead their own.
- **Seen failing**, each by copying the file aside, reverting the fix, running the test, copying it
  back (`cmp` identical):
  - the name check (`named_as` always agreeing): `view_pager::open_refuses_…` (the `.command`
    PDF opened) and `named_as_goes_by_the_real_names_last_extension_in_any_case`;
  - the `/dev` and `/proc` check off: `view_pager::open_refuses_…` (`/dev/stdin` redirected from a
    file launched) and `a_files_place_on_disk_is_its_real_path_and_only_a_regular_files`;
  - the regular-file check off: `view_pager::open_refuses_…` (the pipe said otherwise);
  - the path as given rather than resolved: `view_pager::open_a_files_pages_hands_its_real_path_to_the_opener`
    (`…/link.pdf` launched, not `/private/var/…/the docs/-a 'quoted' report.pdf`);
  - the SSH check off: `view_pager::open_refuses_…`;
  - `setsid` put in a group of its own: `with_setsid_the_opener_leads_no_group_…`;
  - no limit of 4: `at_most_four_launches_are_under_way_and_the_fifth_waits`;
  - the opener not waited for (its thread answering without `wait`): `a_launch_is_said_handed_over_after_five_seconds_and_reaped_when_it_ends`
    (a zombie left);
  - `o` not a key: `o_opens_the_file_from_the_page_the_reader_is_on`.
- **A first run's surprise**: at 200 columns two pages showed side by side, so the e2e test runs
  at 100, as the copy tests do.
- The layering test (`the_viewer_names_no_backend`) caught a doc comment in `launch.rs` naming
  `source::BACKENDS`; reworded.

## Task 2: `o` in a list

- **`Container::on_disk`** returns a `Result` (the real path, or why none), not an `Option`, its
  default the archives' message; `dir.rs` resolves its path with `source::real` and checks it is
  still a directory. The message was first `it is an archive or a folder in one, with no folder of
  its own to show`, too long for a 100-column status line beside the place: now `it is an archive's
  folder, not one of its own to show`.
- **The list's tests use the tar backend**, its `tar` and `gzip` helpers made `pub(crate)` in
  `tests/it/tar.rs` (a file not in the task's *Files:*), since `flate2` is named only by the tar and
  SVG backends and their tests; a plain tar needs no crate. A broken tile is a PDF that isn't one:
  PDFs are opened when their tiles are wanted, so it keeps its tile.
- **tar's names** (`gz`, `bz2`, `xz`, and `tar`) came with task 1, which gave every format its
  names; this task tests them on a file compressed alone.
- **Seen failing**, each reverted as in task 1:
  - `Pager::replace` making its `Pages` with no file: `view_pager::open_a_document_laid_out_again`;
  - `go_on_into` (a tile opened when wanted, gone into) with no file, and the list's `o` not a key,
    a directory with no folder of its own, an archive's folder given its path, `gz` not a gzip's
    name: `view_pager::open_in_a_list_opens_the_tile_chosen_and_refuses_an_archives`, each;
  - the list's `o` not a key: `o_opens_the_file_from_the_page_the_reader_is_on_or_the_tile_chosen`.
- **Left open**: its manual check (in kitty on macOS, `clv tests/data`, `o` on a PDF's tile opens
  Preview, on a folder Finder shows it) waits for the owner, so its box stays unticked; no test may
  launch a real app.

## Codex review fixes

- **`xdg-open` never runs without `setsid`.** A Codex review of task 1 found that, in a process
  group of its own, the opener keeps `clv`'s controlling terminal: an app for the terminal that it
  starts can open `/dev/tty` and write raw bytes to `clv`'s screen, `TOSTOP` being off as it usually
  is (reproduced on a pseudo-terminal), against "nothing raw reaches the terminal". So
  `XdgOpen::command` refuses without `setsid` on `PATH` (`there is no setsid here to keep clv's
  terminal from its app`), and the fallback to `process_group(0)` is gone. macOS's `open` is
  unchanged: it hands the app to launchd, outside `clv`'s session, and itself writes only to its
  null stderr (said in ADR-0034). The spec's 🟡 on `setsid` missing says so.
- **Seen failing**: `a_launch_command_has_the_path_as_one_argument_and_its_program_found_on_path`,
  changed to expect the refusal, failed against the fallback (`left == right`), then passed with
  the fix. The group test lost its case without `setsid` and is now
  `with_setsid_the_opener_leads_no_group_and_open_leads_its_own`. No pseudo-terminal test of
  `/dev/tty`: on macOS `xdg-open` never applies, and Linux CI has no desktop to start an app.
