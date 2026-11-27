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
      # The payload's base SHA can lag behind the base branch; this is its tip when the event fired.
      BASE_SHA: ${{ github.sha }}
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
            "${BASE_SHA}:refs/judge/base" "refs/pull/${PR_NUMBER}/head:refs/judge/head"
          if [ "$(git -C "${repo}" rev-parse refs/judge/head)" != "${HEAD_SHA}" ]; then
            echo "The pull request head changed before fetch; the run for the new head judges it." >&2
            exit 1
          fi
      - name: Collect approvals of the head
        env:
          GH_TOKEN: ${{ github.token }}
        run: |
          pr="repos/${GITHUB_REPOSITORY}/pulls/${PR_NUMBER}"
          {
            gh api --paginate "${pr}/commits?per_page=100" \
              --jq '.[] | (.author.login, .committer.login) | select(. != null)'
            printf '%s\n' "${PR_AUTHOR}"
          } | jq -R . | jq -s unique > "${RUNNER_TEMP}/authors.json"
          gh api --paginate "${pr}/reviews?per_page=100" \
            --jq '.[] | select(.user != null) | [.user.login, .state, .commit_id] | @tsv' |
            while IFS=$'\t' read -r login state commit; do
              permission="$(gh api "repos/${GITHUB_REPOSITORY}/collaborators/${login}/permission" --jq .permission)"
              jq -n --arg login "${login}" --arg state "${state}" --arg commit "${commit}" \
                --arg permission "${permission}" '{$login, $state, $commit, $permission}'
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
          printf '\n::%s::\n' "${marker}"
          exit "${code}"
"#;
