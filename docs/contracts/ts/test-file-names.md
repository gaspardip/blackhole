# ts/test-file-names

The rule checks selected TypeScript and JavaScript paths even if the source
does not parse. Each finding starts at line 1, column 1.

## Catches

- A final `.specs` or `.tests` basename suffix. Use `<subject>.test` or
  `<subject>.spec` (`enforces-the-rule-through-the-repository-configuration-01`,
  `rejects-02`).
- A basename that ends in the word `fixture` or `fixtures` after a dash,
  underscore or dot, or consists of the word alone. Use a nonempty subject
  followed by `.fixture` (`rejects-03` through `rejects-07`).
- A final dot suffix of `test-helper`, `test-helpers`, `test-util`,
  `testutil`, `testutils`, `test-utilities`, `helper`, `helpers`,
  `mock`, `mocks`, `stub`, `stubs` or `harness`. Use
  `<subject>.test-utils` (`rejects-08` through `rejects-11`,
  `other-test-utility-suffixes`).
- A selected script under `tests` or `__tests__` within its nearest
  package, or within the repository when there is no package. The package
  begins at the nearest directory with a `package.json` that the exclusion policy permits,
  even when the manifest is outside the paths passed to `check`. A path
  can receive both a name and a directory finding (`rejects-12`,
  `rejects-13`, `reports-both-name-and-directory-01`,
  `naming_uses_the_whole_tree_and_reports_selected_paths`).

## Does not catch

- Accepted names such as `<subject>.test`, `<subject>.spec`,
  `<subject>.fixture` and `<subject>.test-utils`, including a subject with
  a qualifier such as `admission.browser.test.tsx` (`accepts-02` through
  `accepts-07`, `accepts-13`).
- Paths under `test`, `fixtures`, or a `tests` or `__tests__` ancestor
  above the nearest package (`accepts-08`, `accepts-11`, `accepts-12`,
  `ignores-test-named-ancestors-outside-the-owning-package-01`).
- Other suffixes, non-script paths, and paths excluded from the scan.
- Test contents, test framework configuration, and whether a test sits
  beside the implementation it exercises.
