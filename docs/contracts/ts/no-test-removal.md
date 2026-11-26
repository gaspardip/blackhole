# ts/no-test-removal

Blocking in `blackhole ci` under the `tests` gate.

## Catches

The judge starts with files that the base commit judges. A file qualifies
when a dot-delimited segment of its name is `test`, `spec`, `fixture`,
`mock`, `setup` or `commands`, or when a directory segment is `__tests__`,
`test`, `tests`, `testing` or `test-utils`.

The gate reports a qualifying file that disappears, including a rename
into a base-excluded path. For a file that remains, it compares test cases
and assertions separately. It follows renames reported by Git and uses
the old path's script syntax if the destination has no recognized script
extension. Source that cannot be parsed contributes zero to both counts.
A malformed script renamed to a path without a recognized script extension
still produces a parse finding when both paths are judged.

The count covers these forms in parsed JavaScript and TypeScript. An
invocation site can be a call, an optional call, a `new` expression or a
tagged template.

1. Test cases are invocation sites whose resolved runner starts with `it`,
   `test` or `specify`. The runner recognizes globals and supported exports
   from `vitest`, `@jest/globals`, `node:test` and `@playwright/test`, including
   aliases the resolver can follow. A recognized `todo` modifier or truthy
   `todo` runner option excludes the site. Direct member invocations of
   `each`, `for` and `extend` are factories and are excluded. A parameterized
   test registration counts once at its invocation site.
2. Assertions are invocation sites whose resolved runner starts with a name
   beginning with `expect` or `assert`, including Node's `assert` and strict
   assert modules. When the resolver finds no runner, a site also counts
   if its callee is a bare identifier with either prefix, except for the
   exact names `expect` and `assert`. The judge counts each source position
   once across the file, including top-level assertions and assertions in
   helpers.
3. The expectation builders `expect(...)`, `expect.soft(...)` and
   `expect.poll(...)` are excluded. A standalone bare `expect` on a
   recognized literal counts. The static calls `expect.assertions`,
   `expect.hasAssertions`, `expect.any`, `expect.anything`,
   `expect.arrayContaining`, `expect.arrayOf`, `expect.objectContaining`,
   `expect.stringContaining`, `expect.stringMatching` and `expect.closeTo`
   are excluded.

## Approval

The same approval clears the `tests` gate as for a protected change.
Someone with write access who authored none of the change must approve
the exact head commit in the supplied `--approvals` file. Their latest
non-comment review must be that approval. Findings such as parse errors
are judged separately.

## Does not catch

- An emptied test callback when the recognized call stays and the file's
  assertion count stays the same.
- An assertion weakened or replaced with another counted assertion.
- A test or assertion replaced, moved, or offset by another counted site
  while both counts stay level in each judged file. Moving a test between
  judged files still requires approval if the source file loses a count.
- Test execution, runtime case totals, and custom runners or assertions
  outside the recognized forms above.
