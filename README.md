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

CI runs the same scan only for people with write access. Before merging a pull
request from anyone else, scan it locally:

```sh
git fetch origin pull/<number>/head && python3 -I tools/name_guard.py range origin/main FETCH_HEAD
```

## License

MIT
