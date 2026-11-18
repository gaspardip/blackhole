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

Each rule needs a unique id, reject and accept fixtures, a root-cause fix and provenance.
Run `BLACKHOLE_BLESS=1 ~/.agents/bin/heavy run -- cargo test --test fixtures` to update expectations, then review the diff.

The ledger at `.blackhole/ledger` records existing findings by rule and path.
Only `blackhole ledger` writes it. Allowances can only shrink.

Only the root `blackhole.toml` sets repository-relative `exclude` globs.
File an overage with `blackhole object <rule> <path> <reason>`. Objections last through day 30 and accept one renewal with `--renew`.
`blackhole ci --base <sha> --head <sha>` judges behind, protected, objections, ledger and findings from commit objects.
The `judge` workflow runs it for a pull request when it opens and on each push and retarget, through `.github/judge.sh` and a binary both read from the default branch as it is at that moment, against the policy of the branch the pull request targets. The verdict is a commit status named for that branch, `judge (main)`, because a job in a pull request can take any name and a status cannot be set from one. Require that name, from GitHub Actions, on a branch that must be up to date before merging. A target branch whose name holds anything but lower-case letters, digits, `.`, `_`, `/` and `-` is refused. A change to `.github/`, `.blackhole/` or `blackhole.toml`, a ledger that grows, or a file that arrives under an excluded path while a judged one leaves stays red there, because a status cannot follow a review that is withdrawn later. Whoever may bypass the status merges such a change, and that merge is the approval. The workflow passes no `--approvals`. That flag is for a gate that reads approvals at the moment of merging.
Add an evasion case with base/ and head/ trees, an expect file listing every red gate in sorted order or green, and optional approvals.json. Use .keep for an empty tree.
