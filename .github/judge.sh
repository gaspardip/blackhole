#!/usr/bin/env bash
# Judges every open pull request whose head is HEAD_SHA and reports each verdict as a commit status named for the branch it targets.
set -euo pipefail

api="repos/${GITHUB_REPOSITORY}"

status() {
  gh api -X POST "${api}/statuses/${HEAD_SHA}" -f context="judge ($1)" -f state="$2" -f description="$3" \
    -f target_url="${GITHUB_SERVER_URL}/${GITHUB_REPOSITORY}/actions/runs/${GITHUB_RUN_ID}" > /dev/null
}

# Two pull requests can share a head, and a pull request can change its target after the event. So the judge reads what is open now.
pulls="${RUNNER_TEMP}/pulls.tsv"
gh api --paginate -X GET "${api}/pulls" -f state=open -f per_page=100 |
  jq -r --arg head "${HEAD_SHA}" '.[] | select(.head.sha == $head) | [.number, .base.ref, .user.login] | @tsv' > "${pulls}"
if [ ! -s "${pulls}" ]; then
  echo "judge: no open pull request has the head ${HEAD_SHA}"
  exit 0
fi

git init -q repo
cd repo
git remote add origin "${GITHUB_SERVER_URL}/${GITHUB_REPOSITORY}.git"
refs=("+refs/heads/${DEFAULT_BRANCH}:refs/remotes/origin/default")
while IFS=$'\t' read -r number base _ <&3; do
  # A run that dies leaves this behind, never an older verdict.
  status "${base}" pending judging
  refs+=("+refs/heads/${base}:refs/remotes/base/${number}" "+refs/pull/${number}/head:refs/remotes/head/${number}")
done 3< "${pulls}"

# A private repository refuses a fetch without the token. It goes in by environment, so it reaches neither the process list nor a config file.
auth="$(printf 'x-access-token:%s' "${GH_TOKEN}" | base64 -w0)"
echo "::add-mask::${auth}"
GIT_CONFIG_COUNT=1 GIT_CONFIG_KEY_0="http.${GITHUB_SERVER_URL}/.extraheader" GIT_CONFIG_VALUE_0="AUTHORIZATION: basic ${auth}" \
  git fetch -q origin "${refs[@]}"
# The judge is built from the default branch. A head arrives as git objects and is never checked out or run. Neither the build nor the judge needs the token, so neither gets it.
git checkout -q refs/remotes/origin/default
env -u GH_TOKEN cargo build --locked --release --quiet

verdicts="${RUNNER_TEMP}/verdicts.tsv"
while IFS=$'\t' read -r number base author <&3; do
  work="${RUNNER_TEMP}/${number}"
  mkdir "${work}"
  gh api --paginate --slurp "${api}/pulls/${number}/reviews" > "${work}/reviews.json"
  # Only an approval needs its reviewer's permission. A failed lookup counts as no access.
  jq -r '[.[][] | select(.state == "APPROVED") | .user.login // empty] | unique | .[]' "${work}/reviews.json" |
    while IFS= read -r login; do
      level="$(gh api "${api}/collaborators/${login}/permission" --jq .permission || echo none)"
      jq -n --arg login "${login}" --arg level "${level}" '{($login): $level}'
    done | jq -s 'add // {}' > "${work}/permissions.json"
  gh api --paginate --slurp "${api}/pulls/${number}/commits" > "${work}/commits.json"
  # Whoever wrote or committed a commit cannot be the one who approves it.
  jq --arg author "${author}" --slurpfile permissions "${work}/permissions.json" --slurpfile commits "${work}/commits.json" '{
    authors: ([$author] + [$commits[0][][] | .author.login, .committer.login | select(. != null)] | unique),
    reviews: [.[][] | (.user.login // "") as $login | {
      login: $login,
      state: .state,
      commit: (.commit_id // ""),
      permission: ($permissions[0][$login] // "none")
    }]
  }' "${work}/reviews.json" > "${work}/approvals.json"
  total="$(gh api "${api}/pulls/${number}" --jq .commits)"
  listed="$(jq '[.[][]] | length' "${work}/commits.json")"
  approvals=()
  if ! git diff --quiet refs/remotes/origin/default "${HEAD_SHA}" -- .github/workflows/review.yml; then
    # A review reaches the judge through that workflow as the pull request has it, so one that changes it could keep a withdrawn approval alive.
    echo "judge: the review workflow of #${number} differs from the default branch, so approvals are ignored"
  elif [ "${listed}" != "${total}" ]; then
    # The commit list stops at 250, and an author it leaves out could approve.
    echo "judge: #${number} lists ${listed} of ${total} commits, so approvals are ignored"
  else
    approvals=(--approvals "${work}/approvals.json")
  fi
  state=success
  echo "judge: #${number} against ${base}"
  env -u GH_TOKEN target/release/blackhole ci --base "refs/remotes/base/${number}" --head "${HEAD_SHA}" "${approvals[@]}" || state=failure
  printf '%s\t%s\n' "${base}" "${state}" >> "${verdicts}"
done 3< "${pulls}"

# Two pull requests with one head and one target share a status. A failure sorts first, so it is the one reported.
LC_ALL=C sort "${verdicts}" | awk -F'\t' '!seen[$1]++' | while IFS=$'\t' read -r base state; do
  status "${base}" "${state}" "blackhole ci"
done
