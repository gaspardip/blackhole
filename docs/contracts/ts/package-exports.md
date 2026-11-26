# ts/package-exports

The rule checks every selected file named `package.json`. It reports at
line 1, column 1 of the manifest.

## Catches

- A manifest the JSON decoder rejects, or a top-level value other than an object
  (`invalid-json`, `non-object-manifest`).
- An `exports` string target consisting of `*` or `*.<script extension>`
  at the package root or directly under `src`. Script extensions are
  `js`, `jsx`, `ts` and `tsx`, including their `c` or `m` prefixed forms.
  The rule ignores empty and dot segments and collapses matched parent
  segments before checking the target
  (`rejects-root-source-wildcard-through-nested-conditions-01` through
  `rejects-root-source-wildcard-through-nested-conditions-06`,
  `finds-the-owning-package-when-lint-runs-from-a-nested-directory-01`,
  `exports_reads_the_selected_tree`).
- These targets at any depth under `exports`, including conditional
  objects and fallback arrays. One finding lists the distinct original
  strings in lexical order
  (`rejects-root-source-wildcard-through-nested-conditions-01`,
  `checks-fallback-arrays-while-respecting-null-exclusions-01`,
  `exports_reports_invalid_manifests_and_root_wildcards`).

## Does not catch

- Explicit entrypoints, narrow wildcards such as `./src/schema/*.ts` or
  `./src/*/index.ts`, targets under output directories such as `dist`,
  and extension-restricted asset globs
  (`allows-explicit-entrypoints-namespaces-and-assets-01` through
  `allows-explicit-entrypoints-namespaces-and-assets-06`).
- Private `imports` aliases, a missing `exports` property, and
  `null` targets (`allows-private-aliases-and-packages-without-exports-01`,
  `blocked-exports`).
- Other non-string values under `exports`, such as numbers or booleans.
- Export keys, duplicate JSON keys overwritten during parsing, and
  targets starting or ending with a slash. The rule checks the shape of
  target strings and does not resolve conditions or check whether
  targets exist, load, or stay inside the package.
- Manifests outside the command's selection, including paths omitted
  by the root `exclude` patterns.
