# blackhole

Rules that coding agents cannot talk their way around. A Rust CLI (`blackhole`, crate `blackhole-check`) with its own rule engine.

## Verify

`bin/check` defines green for the code: the `check` workflow runs that script and nothing else. A pull request also has to pass `name-guard`, which scans its commits, branch name, title and body.

## Setup

Once per clone: `git config core.hooksPath .githooks`. The hooks need the list of names from outside the repository (`BLACKHOLE_BANNED`, or the file named by `BLACKHOLE_BANNED_FILE`) and block the commit without it.

Git 2.45 or newer: every git call the tool makes passes `--no-lazy-fetch`, and an older git stops on the unknown option.

## What the local commands promise

`check`, `ledger` and `object` write only inside the checkout, take policy only from `blackhole.toml`, `.blackhole/ledger` and `.blackhole/objections` inside it, and run no program the checkout's git configuration names. They refuse to run when the root git reports is not the directory that holds the nearest `.git`.

Which untracked files they look at is git's answer: its standard ignore rules apply wherever git keeps them (`.gitignore`, the git directory's `info/exclude`, the user's `core.excludesFile`). An ignore rule never hides a tracked file, and `ci` reads commits, where no ignore rule applies.

Git keeps its own files in the git directory the checkout names. That directory can lie outside the checkout (a linked worktree, a submodule, `--separate-git-dir`), and git may refresh the timestamps of its index files there when it reads them. That is git's bookkeeping, not a write this tool makes.

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

Each rule needs a unique id, reject and accept fixtures, a root-cause fix and provenance.
Run `BLACKHOLE_BLESS=1 ~/.agents/bin/heavy run -- cargo test --test fixtures` to update expectations, then review the diff.
Explain any changed line in `tests/corpus/*.snapshot` in the pull request.

The ledger at `.blackhole/ledger` records existing findings by rule and path.
Only `blackhole ledger` writes it. Allowances can only shrink.

Only the root `blackhole.toml` sets repository-relative `exclude` globs.
File an overage with `blackhole object <rule> <path> <reason>`. Objections last through day 30 and accept one renewal with `--renew`.
`blackhole ci --base <sha> --head <sha>` judges behind, protected, objections, ledger, findings, test removals and configured changed-line coverage from commit objects.
A test file deletion or a drop in its test case or assertion count needs the same approval as a protected change. See the [test removal contract](docs/contracts/ts/no-test-removal.md).
Standard repository judge files and base-configured `[guard] paths` also require approval.
These changes are protected: a change to `.github/`, `blackhole.toml` or anything under `.blackhole/` except the ledger; a change to the ledger that leaves it unreadable or allowing more than the base does once renames are followed; a file that arrives under an excluded path while a judged one leaves. A protected change is red unless `--approvals <file>` shows an approval of the head commit by someone with write access who wrote none of the change: that person's last review in the file, comments aside, must approve the head. See [GitHub judge](README.md#github-judge) for how this repository publishes its verdict.
Add an evasion case with base/ and head/ trees, an expect file listing every red gate in sorted order or green, and optional approvals.json. A case-level lcov.info supplies external coverage evidence at reports/lcov.info for the head. Use .keep for an empty tree.
