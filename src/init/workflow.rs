pub(crate) const PATH: &str = ".github/workflows/blackhole.yml";

/// The judge workflow for the running version.
pub(crate) fn content() -> String {
    TEMPLATE.replace("@VERSION@", env!("CARGO_PKG_VERSION"))
}

const TEMPLATE: &str = r#"# Written by blackhole init. It judges every pull request with the base branch's copy of this file.
name: blackhole

on:
  pull_request_target:
    types: [opened, reopened, synchronize, edited]

permissions: {}

defaults:
  run:
    shell: bash

jobs:
  blackhole:
    name: blackhole
    runs-on: ubuntu-24.04
    timeout-minutes: 15
    permissions:
      contents: read
      pull-requests: read
    env:
      PR_NUMBER: ${{ github.event.pull_request.number }}
      PR_AUTHOR: ${{ github.event.pull_request.user.login }}
      HEAD_SHA: ${{ github.event.pull_request.head.sha }}
      # The payload's base SHA can lag behind the base branch, so the fetch step reads the branch's tip.
      BASE_REF: ${{ github.event.pull_request.base.ref }}
    steps:
      - uses: actions/setup-node@949feb2413d6458794dcd2491c4babbbce0c15c1 # v7.1.0
        with:
          node-version: '24'
          token: ''
          package-manager-cache: false
      - name: Install blackhole @VERSION@
        run: |
          tool="${RUNNER_TEMP}/blackhole"
          mkdir "${tool}"
          cd "${tool}"
          printf '{"private": true}\n' > package.json
          npm install --ignore-scripts --save-exact --no-audit --no-fund @gaspardip/blackhole@@VERSION@
          # The registry signs each version's integrity hash, so a swapped tarball fails here.
          npm audit signatures
          test "$(node_modules/.bin/blackhole --version)" = "blackhole @VERSION@"
      - name: Fetch the base and head commits without checking them out
        env:
          GH_TOKEN: ${{ github.token }}
        run: |
          repo="${RUNNER_TEMP}/judge.git"
          git init -q --bare "${repo}"
          auth="$(printf 'x-access-token:%s' "${GH_TOKEN}" | base64 -w0)"
          # The token reaches git through the environment of this one command and is never written to disk.
          GIT_CONFIG_COUNT=1 GIT_CONFIG_KEY_0=http.extraHeader GIT_CONFIG_VALUE_0="Authorization: basic ${auth}" \
            git -C "${repo}" fetch -q --no-tags "${GITHUB_SERVER_URL}/${GITHUB_REPOSITORY}.git" \
            "refs/heads/${BASE_REF}:refs/judge/base" "refs/pull/${PR_NUMBER}/head:refs/judge/head"
          if [ "$(git -C "${repo}" rev-parse refs/judge/head)" != "${HEAD_SHA}" ]; then
            echo "The pull request head changed before fetch; the run for the new head judges it." >&2
            exit 1
          fi
          printf 'BASE_SHA=%s\n' "$(git -C "${repo}" rev-parse refs/judge/base)" >> "${GITHUB_ENV}"
      - name: Collect approvals of the head
        env:
          GH_TOKEN: ${{ github.token }}
        run: |
          pr="repos/${GITHUB_REPOSITORY}/pulls/${PR_NUMBER}"
          commits="$(
            gh api --paginate "${pr}/commits?per_page=100" |
              jq -cs '[.[][] | {sha, author: .author.login, committer: .committer.login}]'
          )"
          api_shas="$(jq -r '.[].sha' <<< "${commits}" | LC_ALL=C sort -u)"
          fetched_shas="$(
            git -C "${RUNNER_TEMP}/judge.git" rev-list refs/judge/base..refs/judge/head | LC_ALL=C sort -u
          )"
          unlinked="$(jq 'any(.[]; (.author | type) != "string" or (.committer | type) != "string")' <<< "${commits}")"
          reason=
          if [[ "${api_shas}" != "${fetched_shas}" ]]; then
            reason="the pull request commit list could not be confirmed"
          elif [[ "${unlinked}" = true ]]; then
            reason="a commit author is not linked to a GitHub account"
          fi
          if [[ -n "${reason}" ]]; then
            printf '%s\n' '{"authors":[],"reviews":[]}' > "${RUNNER_TEMP}/approvals.json"
            printf 'Approvals were ignored because %s.\n' "${reason}"
            exit 0
          fi
          jq --arg author "${PR_AUTHOR}" '[.[] | .author, .committer] + [$author] | unique' \
            <<< "${commits}" > "${RUNNER_TEMP}/authors.json"
          gh api --paginate "${pr}/reviews?per_page=100" |
            jq -rs --arg head "${HEAD_SHA}" '
              [.[][] | select(.user != null and .state != "COMMENTED")]
              | group_by(.user.login | ascii_downcase)[]
              | last | select(.state == "APPROVED" and .commit_id == $head) | .user.login
            ' |
            while IFS= read -r login; do
              if response="$(gh api -i "repos/${GITHUB_REPOSITORY}/collaborators/${login}/permission")"; then
                permission="$(sed '1,/^[[:space:]]*$/d' <<< "${response}" | jq -er '.permission | strings')"
              else
                IFS=' ' read -r protocol status _ <<< "${response}"
                [[ "${protocol}" = HTTP/* && "${status}" = 404 ]] || exit 1
                permission=none
              fi
              jq -n --arg login "${login}" --arg commit "${HEAD_SHA}" \
                --arg permission "${permission}" '{$login, state: "APPROVED", $commit, $permission}'
            done |
            jq -s --slurpfile authors "${RUNNER_TEMP}/authors.json" '{authors: $authors[0], reviews: .}' \
              > "${RUNNER_TEMP}/approvals.json"
      - name: Judge the head
        run: |
          code=0
          (cd "${RUNNER_TEMP}/judge.git" && "${RUNNER_TEMP}/blackhole/node_modules/.bin/blackhole" ci \
            --base "${BASE_SHA}" --head "${HEAD_SHA}" --approvals "${RUNNER_TEMP}/approvals.json") \
            > "${RUNNER_TEMP}/judge.log" 2>&1 || code=$?
          # Paths in diagnostics can resemble workflow commands.
          marker="$(cat /proc/sys/kernel/random/uuid)"
          printf '::stop-commands::%s\n' "${marker}"
          cat "${RUNNER_TEMP}/judge.log"
          if [ "${code}" -eq 0 ]; then
            printf '\nblackhole ci: green\n'
          fi
          printf '\n::%s::\n' "${marker}"
          exit "${code}"
"#;

pub(super) const REVIEW_PATH: &str = ".github/workflows/blackhole-review.yml";
pub(super) const RERUN_PATH: &str = ".github/workflows/blackhole-rerun.yml";

pub(super) const REVIEW: &str = r#"name: blackhole review

on:
  pull_request_review:
    types: [submitted, dismissed]

permissions: {}

jobs:
  relay:
    runs-on: ubuntu-24.04
    steps:
      - run: "true"
"#;

pub(super) const RERUN: &str = r#"name: blackhole rerun

on:
  workflow_run:
    workflows: ["blackhole review"]
    types: [completed]

permissions: {}

jobs:
  rerun:
    runs-on: ubuntu-24.04
    timeout-minutes: 20
    permissions:
      actions: write
    steps:
      - shell: bash
        env:
          GH_TOKEN: ${{ github.token }}
          HEAD_SHA: ${{ github.event.workflow_run.head_sha }}
          HEAD_BRANCH: ${{ github.event.workflow_run.head_branch }}
          HEAD_REPOSITORY: ${{ github.event.workflow_run.head_repository.full_name }}
          LC_ALL: C
        run: |
          if [[ ! "${HEAD_SHA}" =~ ^[0-9a-f]{40}$ ]]; then
            echo "Invalid review head SHA." >&2
            exit 1
          fi
          if [[ -z "${HEAD_BRANCH}" || "${HEAD_BRANCH}" == *[$'\r\n']* ||
                ! "${HEAD_REPOSITORY}" =~ ^[a-zA-Z0-9_.-]+/[a-zA-Z0-9_.-]+$ ]]; then
            echo "Invalid review head branch or repository." >&2
            exit 1
          fi
          actions="repos/${GITHUB_REPOSITORY}/actions"
          id=$(
            gh api --paginate "${actions}/workflows/blackhole.yml/runs?event=pull_request_target&head_sha=${HEAD_SHA}&per_page=100" |
              jq -rs --arg sha "${HEAD_SHA}" --arg branch "${HEAD_BRANCH}" --arg repo "${HEAD_REPOSITORY}" '
                [.[].workflow_runs[] | select(
                  .head_sha == $sha and .head_branch == $branch and .head_repository.full_name == $repo
                )] | max_by([.created_at, .id]) | .id // empty
              '
          )
          if [[ -z "${id}" ]]; then
            echo "No blackhole judge run found for this head."
            exit 0
          fi
          run="${actions}/runs/${id}"
          while true; do
            status=$(gh api "${run}" --jq .status)
            [[ "${status}" == completed ]] && break
            sleep 5
          done
          for attempt in 1 2 3; do
            if gh api --method POST "${run}/rerun" >/dev/null 2>&1; then
              exit 0
            fi
            if [[ "${attempt}" -lt 3 ]]; then
              sleep "${attempt}"
            fi
          done
          echo "The blackhole judge was not refreshed and must be rerun by hand." >&2
          exit 1
"#;
