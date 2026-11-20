# blackhole

Rules that coding agents cannot talk their way around.

Rico Mariani's "pit of success" asks that the wrong thing be annoying but not
impossible. blackhole removes "not impossible": no disable comments, no rule
that can be switched off, and a judge the agent cannot edit.

Status: pre-release. Nothing is published yet.

## Working on this repo

```sh
git config core.hooksPath .githooks
```

The hooks run `tools/name_guard.py`, which needs a list of names kept outside
the repository (`BLACKHOLE_BANNED`, or the file named by `BLACKHOLE_BANNED_FILE`).
Without the list the guard refuses to run and the commit is blocked.

## Packaging the CLI

`blackhole-check` installs the `blackhole` binary from Rust source. Run
`cargo package --locked` to build and verify the crate.

The npm package `@gaspardip/blackhole` bundles the same CLI and requires
Node.js 24 or later. Both distributions require Git 2.45 or later on the user's PATH.
The prebuilt targets are:

| Platform | x64 | Arm64 |
| --- | --- | --- |
| Linux (glibc) | `x86_64-unknown-linux-gnu` | `aarch64-unknown-linux-gnu` |
| macOS | `x86_64-apple-darwin` | `aarch64-apple-darwin` |

For each target, run `cargo build --locked --release --target <triple>`
from the same release commit. Collect each executable at
`target/<triple>/release/blackhole`. Build on the oldest OS or sysroot
you intend to support and test on that baseline.

From the repository root, stage the four binaries in a fresh directory
and pack them into one tarball:

```sh
python3 tools/stage_npm.py target target/npm-package
pnpm --dir target/npm-package pack --pack-destination ..
```

The package version comes from Cargo metadata. For a local single-target
install test, add `--smoke-target <triple>` to the staging command and use a
fresh destination. That package is marked private. Install the resulting
tarball in a clean project with `pnpm add --offline /path/to/package.tgz`,
then run `pnpm exec blackhole --version` and `pnpm exec blackhole check`
in a Git repository.

The `packaging` workflow runs on packaging pull requests and manually for a
selected ref. It builds on Ubuntu 22.04 and macOS 15 for both architectures.
These runners are provisional test environments. Each runner verifies the
same tarball by installing offline with scripts disabled and running the CLI.
After all four pass, the `distribution-<commit SHA>` workflow artifact holds
the tested npm tarball, verified crate and `SHA256SUMS` for release.

## Repository checks

`blackhole ci` requires the existing independent head approval for changes to
conventional TypeScript, lint, formatter, test, coverage and Git hook configs,
including `package.json` and conventional local ESLint rule directories. For
custom judge files, list repository-relative globs in the base configuration.
Include imported helpers and symlink targets when they live elsewhere.

```toml
[guard]
paths = ["tools/lint/**"]

[coverage]
lcov = "reports/lcov.info"
```

With coverage enabled on the base, CI compares the two commits and requires
a hit on each added or modified instrumented line in recognized non-test
source and scripts with a shebang. It uses base exclusions even when the head
has an approval. Deleted lines, pure renames, TypeScript declarations and test
files have no coverage requirement. A move into production source from a test
or excluded path counts as a new file.

The judge reads `<coverage-dir>/<full head SHA>/<lcov>`. Supply the external
directory with `blackhole ci --base <base> --head <head> --coverage-dir <dir>`.
Missing evidence for a changed source file, invalid line records, inconsistent
line totals and uncovered changed lines turn the coverage gate red. LCOV
`SF` paths must be repository-relative. The producer must report every
instrumented line with a `DA` record, including zero hits, and use `LF:0`
for a file with no instrumented lines.

The SHA selects a report. Its provenance and the completeness of its line map
depend on the producer. The GitHub judge still needs a trusted producer that
isolates head code from credentials and from the final evidence. A file uploaded
by a PR job can claim arbitrary hits or omit executable lines. Until that
integration exists, enabling coverage leaves changed source red on GitHub
when the judge runs without evidence.

## GitHub judge

Release one publishes the `blackhole judge` check from the `blackhole-judge` App
only for pull requests targeting `main` in `gaspardip/blackhole`. Adopters run
`blackhole ci` in their own workflow.

On open, reopen, head update or retarget, `judge.yml` builds `blackhole` from the
event's base commit in a job without secrets or token permissions. It fetches
the PR head into a fresh bare Git repository and judges it as commit data.
The verdict excludes review data. The workflow omits `--approvals`, so a
protected change stays red and requires a maintainer merge.

A separate job runs the publisher from the same base commit. It verifies the
current PR head and target branch through the GitHub API. A failed or cancelled
judge job yields a failure check for that head when the publisher runs. The
publisher uses `JUDGE_APP_ID` and `JUDGE_APP_PRIVATE_KEY` to request an
installation token with `checks: write`.

The repository's event policy must allow `pull_request_target` for `judge.yml`.
Require the check from the App's numeric ID and require branches to be up to
date before merging. App attribution depends on exclusive access to the signing
key. Workflows on eligible same-repository branches can access repository-scoped
Actions secrets.

The publisher checks that the base SHA is current before issuing green. GitHub
stores the check on the head SHA, so PRs with the same head share it. A passing
check can outlive a main update already contained in the head or the expiry of
a tracked objection. Objection expiry is evaluated when `blackhole ci` runs.

## License

MIT
