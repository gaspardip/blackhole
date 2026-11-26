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

CI scans PR titles and bodies only when their author has write access, including
when the head is in a fork. It scans PR commits only when the author has write
access and the head is in this repository. Use `bin/merge-outside` for fork PRs
and PRs from people without write access. The script only accepts PRs targeting
`main`, scans the commits, title and body before merging, and checks the landed
squash.

```sh
bin/merge-outside <number>
```

A squash merge copies the title and body into `main`. Text typed into the merge
dialog is scanned only after it lands, so merge without editing the message.

CI does not scan your edit of someone else's issue, pull request or comment
either. Write the new text to a file and scan it before you save it:

```sh
python3 -I tools/name_guard.py message <file>
```

On a pull request from a fork, the review scan fails because it cannot read the
list, so run `python3 -I tools/name_guard.py message <file>` on the review text
before posting it.

The guard stops mistakes. It cannot stop someone with write access who sets out
to publish a name: they could change the workflow on their own branch, or post
the name anywhere else.

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

## Releasing

1. Bootstrap once. Publish the first crate by hand with a short-lived
   crates.io API token and revoke it afterward. Publish the first npm version
   by hand while logged in. Then configure trusted publishers for both packages
   with the repository, workflow filename `release.yml` and environment
   `release`. Allow direct publishing for the npm trusted publisher.
   [crates.io setup](https://crates.io/docs/trusted-publishing) ·
   [npm setup](https://docs.npmjs.com/trusted-publishers/)
2. Before pushing a release tag, create the GitHub `release` environment with
   a required reviewer. The artifact scan reads the same `BANNED_NAMES`
   repository secret as the `name-guard` workflow.
3. For a release candidate, set the package version in `Cargo.toml` and the
   matching package entry in `Cargo.lock` to `X.Y.Z-rc.N`. After that commit
   lands on `main`, tag it `vX.Y.Z-rc.N` and push the tag. The workflow
   publishes to npm's `next` dist-tag and creates a GitHub prerelease.
4. After that prerelease is published, change only those two version values to
   `X.Y.Z`. Once the promotion commit lands on `main`, tag it `vX.Y.Z` and
   push the tag. The workflow verifies the version-only diff from the
   prerelease and publishes to npm's `latest` dist-tag.

To roll back, the maintainer runs
`npm dist-tag add @gaspardip/blackhole@<previous> latest` while logged in
(the configured publisher only publishes) and `cargo yank` for the crate.

## Repository checks

Advisory rules report findings without affecting the verdict.

`boundary/file-size` reports files that exceed a repository-wide line limit.
Set a positive integer in the root `blackhole.toml` to enable the limit.

```toml
[boundaries]
max-lines = 500
```

The limit applies to decoded text, including generated files within the scan.
Root `exclude` patterns apply. In CI, the base limit applies until an
independently approved head supplies a new limit. See the
[file-size contract](docs/contracts/boundary/file-size.md) for counting and
selection details.

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

`blackhole ci` also requires independent head approval when a judged test
file disappears or its test case or assertion count drops. The judge
compares counts per file and follows renames reported by Git. See the
[test removal contract](docs/contracts/ts/no-test-removal.md) for the
recognized forms.

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
tip of `main` when the event fired, in a job without secrets or token permissions. It fetches
the PR head into a fresh bare Git repository and judges it as commit data.
The verdict excludes review data. The workflow omits `--approvals`, so a
protected change or test removal stays red and requires a maintainer merge.

If a head branch name that looks like a commit SHA prevents `pull_request_target`
from running, push the commits to a branch with another name and open a new PR
from it. Renaming the head branch of an open PR closes that PR.

A separate job runs the publisher from the same commit. It verifies the current
PR head, target branch and tip of `main` through the GitHub API and uses an App
installation token with `checks: write`. A failed or cancelled judge job yields
a failure check for that head when the publisher runs.

Configure GitHub in this order. Steps 1 to 5 come before the workflow runs.

1. Create the `judge` environment with a deployment branch rule allowing only
   `main`. The rule matches the default branch (`main`) because
   `pull_request_target` runs on it. Referencing a missing environment makes
   GitHub create it without protection rules, so create it first.
2. Generate a new private key for the App. Add it as `JUDGE_APP_PRIVATE_KEY`,
   and the App ID as `JUDGE_APP_ID`, as secrets in `judge`.
3. Delete the two repository-level secrets and revoke the App's old private key.
   Eligible workflows on other branches can read repository-level secrets, so
   an old key may have been copied. If anyone else had write access while the
   key was a repository secret, register a new App instead and require its ID:
   a check posted with the old key stays valid after the key is revoked.
4. Install the App on `gaspardip/blackhole` only. Without an installation the
   publisher gets no token and posts nothing.
5. Allow `pull_request_target` for `judge.yml` in the repository's event policy.
6. Protect `main`: require a pull request before merging and require PR
   branches to be up to date before merging. Without the pull request rule, a
   writer can push a judged commit straight to `main`, and a PR whose head
   already contains it keeps its old check.
7. Open an up-to-date pull request with a change outside the protected paths and
   wait for the App's `blackhole judge` check to succeed. GitHub offers an App as
   a check's source only after a successful run in the repository within the past
   seven days. Then require the `blackhole judge` check from the App's numeric ID.

The publisher checks the tip of `main` and the judge's UTC date before issuing green.
If the date is missing or from another day, it posts failure. Re-run the whole
judge for a verdict on the current day. GitHub stores the check on the head SHA,
so PRs with the same head share it. A passing check stays valid while the head
and `main` stay the same, even after an objection it relied on expires, so the
PR can still merge days later. Once that objection is on `main` and expired,
later PRs are red under `findings` while the underlying violation remains.

## License

MIT
