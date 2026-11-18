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

CI scans only what people with write access wrote or pushed. For a pull request
from anyone else, or from a fork, the `name-guard` check passes without scanning,
so scan it locally before merging. A squash merge copies the title and body into
`main`, so the command scans those too:

```sh
git fetch origin pull/<number>/head && GUARD_TEXT_PR="$(gh pr view <number> --json title,body,headRefName --jq '.title, .body, .headRefName')" python3 -I tools/name_guard.py range origin/main FETCH_HEAD
```

## License

MIT
