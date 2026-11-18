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
from anyone else, or from a fork, the `name-guard` check passes without scanning.
Merge those with the script that scans the commits, title and body, then merges
exactly what it scanned:

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

`bin/merge-outside` only merges pull requests that target `main`.

The guard stops mistakes. It cannot stop someone with write access who sets out
to publish a name: they could change the workflow on their own branch, or post
the name anywhere else.

## License

MIT
