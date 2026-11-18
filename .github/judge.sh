#!/usr/bin/env bash
# Judges HEAD_SHA against the branch BASE_REF names and reports the verdict as a commit status named for that branch.
set -euo pipefail

# Only the fetch and the report need the token, so nothing else inherits it.
token="${GH_TOKEN}"
unset GH_TOKEN

# The ranges below mean ASCII only in this locale.
export LC_ALL=C
# shortcut: GitHub matches status names without regard to case, so a target another branch could share a status name with is refused; spell such a name out in the status when someone needs one.
case "${BASE_REF}" in
  *[!a-z0-9._/-]*)
    echo "judge: ${BASE_REF} cannot be judged: a target branch may hold only lower-case letters, digits and . _ / -"
    exit 1
    ;;
esac

git init -q repo
cd repo
git remote add origin "${GITHUB_SERVER_URL}/${GITHUB_REPOSITORY}.git"
# A private repository refuses a fetch without the token. It goes in by environment, so it reaches neither the process list nor a config file.
auth="$(printf 'x-access-token:%s' "${token}" | base64 -w0)"
echo "::add-mask::${auth}"
# HEAD is the default branch as it is now, whatever the event of this run says.
GIT_CONFIG_COUNT=1 GIT_CONFIG_KEY_0="http.${GITHUB_SERVER_URL}/.extraheader" GIT_CONFIG_VALUE_0="AUTHORIZATION: basic ${auth}" \
  git fetch -q origin "+HEAD:refs/remotes/origin/default" "+refs/heads/${BASE_REF}:refs/remotes/origin/base" "+refs/pull/${NUMBER}/head:refs/remotes/origin/head"
# The judge is built from the default branch. The head arrives as git objects and is never checked out or run.
git checkout -q refs/remotes/origin/default
cargo build --locked --release --quiet
# No review reaches the judge, because a status cannot follow one that is withdrawn later.
echo "judge: $(git rev-parse --short HEAD) judges ${HEAD_SHA} against ${BASE_REF}. A person approves a protected change by merging it past this status."
state=success
target/release/blackhole ci --base refs/remotes/origin/base --head "${HEAD_SHA}" || state=failure
# A job in a pull request can take any name, but only a token that may write can set a status, and GitHub requires both when they share a name.
GH_TOKEN="${token}" gh api -X POST "repos/${GITHUB_REPOSITORY}/statuses/${HEAD_SHA}" -f context="judge (${BASE_REF})" -f state="${state}" \
  -f description="blackhole ci" -f target_url="${GITHUB_SERVER_URL}/${GITHUB_REPOSITORY}/actions/runs/${GITHUB_RUN_ID}" > /dev/null
[ "${state}" = success ]
