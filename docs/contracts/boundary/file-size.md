# boundary/file-size

## Catches

- A selected file of any extension whose decoded line count exceeds the
  positive integer `[boundaries].max-lines` in root `blackhole.toml`. One
  repository-wide limit applies to all selected files, including generated
  text and the configuration file itself. The repository scanner follows
  file links and reports under the selected path.
- BOM-marked UTF-16 and UTF-32 in either byte order, and UTF-8 otherwise.
  Malformed units become replacement characters, including in binary files.
  LF, CRLF, CR, U+2028 and U+2029 end lines. CRLF is one break, and blank
  lines count. NULs stay in the decoded text when counting, so a NUL
  between CR and LF keeps the two breaks separate.
- The finding starts at column 1 of the first line over the limit and
  includes the total line count and the limit (`endings`).
- In `blackhole ci`, a head file that exceeds the base limit when a changed
  limit has no independent head approval (`raise-limit`).

## Does not catch

- Any file when `max-lines` is omitted. The limit has no default.
- An empty decoded file or a file at or below the limit. A final line
  terminator adds no extra line (`limit`).
- Byte size, line width or separate limits by path.
- Files outside the command's selection, including files matched by the
  root `exclude` patterns. Use these patterns to omit binary files. A local
  scan also skips untracked files ignored by Git. Paths without file
  contents, such as directories and local dangling links, have no line count.
- The root policy files `.blackhole/ledger` and `.blackhole/objections`.
- In `blackhole ci`, a file within the head limit when independent head
  approval makes that limit apply (`approve-limit`).
