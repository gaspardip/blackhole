# blackhole

Rules that coding agents cannot talk their way around. A Rust CLI (`blackhole`, crate `blackhole-check`) with its own rule engine.

## Verify

`bin/check` defines green for the code: the `check` workflow runs that script and nothing else. A pull request also has to pass `name-guard`, which scans its commits, branch name, title and body.

## Setup

Once per clone: `git config core.hooksPath .githooks`. The hooks need the list of names from outside the repository (`BLACKHOLE_BANNED`, or the file named by `BLACKHOLE_BANNED_FILE`) and block the commit without it.

## The name guard

A private list of names must never appear in this repository: file contents, file names, commit messages, branch names, tags, pull request and issue text.

- The hooks and CI run `tools/name_guard.py`. A hit prints `where:line banned name #n` and never the text.
- On a hit, reword. Never pass `--no-verify`, and never print, copy or guess the list.
- Repositories that adopt blackhole privately are "private repo A", "private repo B". Their code is never copied in.
- Scan pull request text before opening it: write the title and body to a file, then `python3 -I tools/name_guard.py message <file>`. Never paste the text into a shell command.

## This repo lives by its own rules

- No `#[allow]`, no `#[expect]`, no loosened lint config. Fix the code.
- Comments say why, in one line, when the code cannot. No narration.
- No new crate without the maintainer's approval.

## Workflow

- `main` is protected. Branch, commit with conventional commits (`fix:`, `feat:`, `refactor:`, `chore:`, `test:`, `docs:`), open one pull request per change. Merges are squashes.
- Tests come first: a new behaviour starts as a failing test.
