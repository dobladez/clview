# ADR-0034: Open a file through the platform's opener, by its real path, only where its name agrees

- **Status:** Accepted
- **Date:** 2026-11-14

## Context

The pager's `o` hands what the reader is looking at to the app the desktop opens it with: a PDF
to fill in, a Word document to edit, a folder in the file manager
([Opening in another app](../usage.md#opening-in-another-app)). Every desktop has a program that
does it: macOS's `open` (Launch Services), freedesktop.org's `xdg-open` on Linux and the BSDs. Each
picks the app by the file's name, its extension, not by what is in it; `clv` picks its backend by
what is in it. And `clv` runs in a terminal, which the opener and the app it starts must neither
write to nor take, and may run over SSH, where an app opened would be on the far machine.

What the pager shows comes from many places: a file named on the command line, a link to one,
`/dev/stdin` (redirected from a file, or a pipe), a file in an archive, a directory's files.

## Decision

- **The platform's opener, run by `std::process`**: a list of our own, `view::launch::LAUNCHERS`,
  each a `Launcher` saying whether it applies (the system and `DISPLAY` or `WAYLAND_DISPLAY`, given
  to it, never read by it), the command, and its exit statuses in words; the first that applies
  opens. To begin with `open` on macOS, and `xdg-open` on Linux and the BSDs with a desktop
  session. The program is found on `PATH` by name, in absolute directories only.
- **The real path, one argument, no shell.** The path is resolved when the file is opened
  (`std::fs::canonicalize`: absolute, links followed), and that is what is checked and launched.
  Absolute, it starts with `/`, so it is never taken for an option; spaces and quotes reach the
  opener as they are. A file with no path of its own is refused: one in an archive, one that isn't
  a regular file (a pipe, a device), and one given or resolved under `/dev` or `/proc`
  (`/dev/stdin`, even redirected from a file).
- **The name agrees with the format.** A file is opened only if its real name ends in an extension
  one of the backend's formats' files go by (`Format::names`), in any case: any of its formats,
  not only the one it was read as, so a `.md` read as HTML still opens. A PDF named
  `report.command` isn't opened, nor a `report.pdf` linked to it: the desktop would run it as a
  script. The status line names what `clv` showed it as in the backend's own words
  (`Backend::shows`).
- **Folders on macOS are revealed, never opened** (`open -R`): Finder takes a folder for a
  package, an app that `open` would launch, by its extension or by a flag in its metadata, so no
  name tells the two apart. `xdg-open` opens a folder in the file manager.
- **Never waited for, never tied to the terminal.** stdin, stdout and stderr are null: nothing it
  writes reaches the screen, and no pipe ties it to `clv` (an app it starts would inherit the pipe,
  and could die of `SIGPIPE` once `clv` is gone). `open` runs in a process group of its own
  (`CommandExt::process_group(0)`), so `Ctrl-C` in `clv` doesn't reach it; it hands the app to
  launchd, which starts it outside `clv`'s session, and itself writes only to its null stderr.
  `xdg-open` runs through `setsid`, so it has no controlling terminal and an app for the terminal
  it starts fails rather than taking `clv`'s, or writing to it through `/dev/tty`. Where `PATH` has
  no `setsid`, `o` is refused, saying so: in a process group of its own, `xdg-open` and what it
  starts would keep `clv`'s controlling terminal, and with `TOSTOP` off, as it usually is, nothing
  stops a write to `/dev/tty`, raw bytes on `clv`'s screen. A thread waits for
  the opener as long as it runs, so it is never left a zombie; the status line says how it went,
  or after 5 s that it was handed over. At most 4 launches are outstanding.
- **Refused over SSH**, by the `local` that `c` goes by
  ([ADR-0009](0009-copy-a-page-through-the-terminals-clipboard-protocol-else-the-systems.md)).

## Evidence

The platforms' documents, and the openers watched from outside:

- `xdg-open`'s manual (xdg-utils): exit status 1 for an error in the command line, 2 for a file
  that doesn't exist, 3 for a tool it needs not found, 4 for an action that failed.
- macOS 26.6.2, `open` as installed: `/usr/bin/open /nonexistent.pdf; echo $?` prints `The file
  /nonexistent.pdf does not exist.` and `1`; `open -R` the same for a missing folder. Every failure
  is 1.
- util-linux's `setsid(1)` manual: it calls fork(2) if it starts as a process group's leader,
  and otherwise runs the program in its own process; without `-w` the forking parent doesn't wait
  for it. So the opener isn't put in a group of its own before `setsid`: it would always fork, and
  the opener's exit status be lost. `setsid(2)` makes the new session's process group too, so
  `Ctrl-C` doesn't reach it either way. Read from the manual, not yet watched on a Linux desktop;
  the tests run a fake `setsid` that runs what it is given.

## Alternatives considered

- **The `open` or `opener` crates**: a dependency for a few lines, with their own lists of openers
  (WSL's among them) we couldn't extend or test as ours, and a choice of shell or not that isn't
  ours.
- **The path as given**: a link's name may say another format than the file it leads to
  (`photo.png` to `photo.command`), and a relative path depends on the opener's directory.
- **Second-guessing the format** (only the format it was read as): a `.md` read as HTML, or a
  `.dot` Word template, is what it says; the rule only stops a dangerous mismatch.
- **`xdg-open` in a process group of its own where there is no `setsid`**, as first written: an
  app for the terminal that it starts opens `/dev/tty` and writes to `clv`'s screen (reproduced on a
  pseudo-terminal in a review, 2026-11-14), against the rule that nothing raw reaches the terminal.
- **`setsid` with `-f`**: it always forks, so the opener's exit status is lost; and calling
  `setsid()` in the child before `exec` takes `unsafe`, which the crate forbids.
- **Waiting for the opener on the pager's thread**: some of `xdg-open`'s ways wait for the app to
  exit, and the keys would stop.
- **A pipe for its stderr**, to say what it said: an app it starts inherits the pipe, and dies of
  `SIGPIPE` writing to it once `clv` has quit. The exit status in the launcher's words says enough.
- **WSL's `wslview`**: it hands the path on to PowerShell or `cmd.exe`, a parser of its own, so it
  waits for tests on a real WSL ([backlog](../backlog.md#more-openers-wsl-among-them)).

## Consequences

- **Positive:** no dependency; each launcher an entry and a test, tested on any machine since none
  reads the environment itself; nothing an app does reaches the screen; a file's name never makes
  it run as something it doesn't look like in `clv`.
- **Accepted:** on a Linux or BSD system with no `setsid` (util-linux and BusyBox have one), `o`
  opens nothing. A file swapped for another between the check and the launch (a race no hand-over
  by path can close) opens as the other: the path is checked when the file is opened, and the
  reader may move it after. On Linux, a link of the reader's to `/dev/stdin` redirected from a file
  resolves to that file, which opens as the file it is. An archive's file can't be opened. Over
  SSH nothing is. A file whose name says nothing (`notes`) isn't opened, however `clv` showed it.
- **Follow-ups:** `O`, choosing the app
  ([backlog](../backlog.md#o-open-a-file-in-an-app-of-the-readers-choosing)), adds a method to
  `Launcher`, with a default; an archive's file unpacked to open
  ([backlog](../backlog.md#open-an-archives-file-in-another-app)); more openers.
- **Revisit when:** a terminal offers to open a file on the reader's machine over SSH; a desktop
  `xdg-open` doesn't reach; a platform with an opener that takes a path some other way.
