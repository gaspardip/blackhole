# boundary/folder-imports

Advisory until the resolution cases under Does not catch are addressed. Findings print with `[advisory]` and leave the verdict unchanged.

## Catches

- Dependencies from a configured source folder into a denied path or
  package name, through static imports, re-exports, dynamic imports,
  TypeScript import-equals and `require` calls (`forms`).
- Relative paths into a denied folder even when the target is missing.
  For resolved imports, a denied written file still counts when extension
  aliasing selects another file (`forms` and
  `extension_alias_preserves_denied_written_path` in `tests/boundary/folders.rs`).
- Denied targets reached through `tsconfig.json` paths and
  `package.json` imports aliases, including aliases to package subpaths
  (`forms`, `package-import-aliases`, `alias-folder` and
  `boundary-folder-package-alias`).
- Package names listed in the deny patterns, and package entry points
  resolved to files under a denied path (`forms`, `listed-package`).
- A re-export chain into a denied folder. Named, default, namespace
  and TypeScript import-equals bindings carry their dependency when
  exported. This includes `export import` and `export =`
  (`reexport-chain`, `local-reexports`,
  `imported_bindings_reexport_their_module`).
- Mixed imports and re-exports check runtime and declaration targets.
  Only the readings for exported bindings pass through to importers.
  A local type-only re-export passes through the declaration target
  even when the binding came from an ordinary import
  (`mixed_reexports_follow_the_exported_type_and_value_readings`,
  `local_type_reexports_follow_the_declaration_reading`).
- Type-only imports, type re-exports and `import()` type queries.
  Package imports and exports use the file's resolution mode or an
  explicit `resolution-mode`. Relative paths and `tsconfig.json` paths
  aliases try the file before the directory, as TypeScript does, so a
  JavaScript file can win over a directory declaration. A local
  directory's `typings` field precedes `types`. An explicit `.jsx`
  specifier can resolve to `.ts`. Extensionless local lookups skip
  `.mts` and `.cts`, which explicit `.mjs` and `.cjs` specifiers reach.
  A bare specifier containing `?` is looked up as an exact file name in
  `node_modules`. For any other bare package, declaration lookup
  follows TypeScript 5.9: matching `exports` conditions in manifest
  order, with `typesVersions` ignored; without `exports`, the first
  matching `typesVersions` entry, then `typings`, `types`, `main` with
  its declaration file, then the index. In NodeNext import mode a legacy
  subpath gets no implicit extension or directory index. JavaScript is
  used, with the same conditions, only when no declaration or
  TypeScript source resolves (`type-across-folder`, `import-types`,
  `type-conditions`, `declaration-main-fields`,
  `type_imports_follow_the_file_or_explicit_resolution_mode` and the
  `type_local_*`, `type_paths_*` and `type_package_*` rows in
  `tests/boundary/folders.rs`).
- Runtime package targets respect the order of matching conditions in
  the manifest. Both `import` and `require` can select `module-sync`
  (`module_sync_is_available_to_both_runtime_loaders`,
  `boundary-folder-module-sync`).
- A static bare TypeScript import can reach the denied `require` target
  even when the `import` target is allowed. The rule checks both conditions
  in a plain `.ts` file without a declared module format
  (`commonjs-package` and
  `static_typescript_import_checks_both_package_conditions` in
  `tests/boundary/folders.rs`).
- Calls through recognized Node loaders, including aliases of `require`,
  `module.require` and loaders made by `createRequire` from `module`
  or `node:module`. Unknown load arguments or untrusted loader bases
  can report “specifier is not a literal” (`forms`, `require-bindings`,
  `require-base`). Empty or unreadable specifiers, file URLs and invalid
  percent escapes also report (`empty-specifiers`, `lone-surrogate`,
  `url-specifiers`). Parentheses and supported TypeScript wrappers
  preserve the finding position (`wrapped-positions`,
  `non_null_wrapped_bases_and_specifiers_keep_their_findings`).
- Whole values of `process` or `globalThis` report when they are
  assigned to a mutable binding, passed to another function, returned,
  or used in a logical expression or container. Direct member reads,
  simple const destructuring and const aliases with trackable uses
  stay clean, as do whole values discarded by an expression statement
  or `void`. Dependencies in discarded expressions are still checked
  (`whole_process_and_global_values_report_at_the_use`,
  `tracked_and_exported_process_and_global_values_stay_clean`,
  `discarded_process_and_global_values_only_check_dependencies`).
- A lookup result passed to a loader or dynamic `import()` reports
  “specifier is not a literal”, including when a const alias carries
  the result (`dynamic_import_of_lookup_reports_non_literal` and
  `allowed_lookup_names_are_still_non_literal_load_arguments` in
  `tests/boundary.rs`).
- In `blackhole ci`, forbidden dependencies in the head commit tree,
  including a path alias, a re-export chain or a package declaration
  target. Removing the policy in the head still leaves the base policy
  in force without approval (`alias-folder`, `boundary-reexport-chain`,
  `boundary-declaration-main-fields`,
  `unapproved_removal_keeps_the_base_folder_policy`). An approved policy
  addition can produce advice even with a historical advisory ledger row
  (`approve-folder`, `approve-folder-ledger`).

## Does not catch

- Imports from outside the configured source patterns, or imports whose
  names and targets are allowed (`allowed`, `package-import-aliases`).
- A workspace package imported by name without a listed install link
  or path alias when only its workspace folder is denied. Its name can be listed in the deny patterns
  (`workspace-name`, `forms`).
- An ordinary import inside an intermediate file that the file keeps
  private. The caller is checked through re-exports
  (`reexport-chain`, `local-reexports`). Assigning an imported value to
  a different local binding before exporting it also keeps the original
  binding private (`imported_bindings_reexport_their_module`).
- Re-exports expressed as CommonJS assignments or as references inside
  newly declared types or values.
- Shadowed `require`, `module` or `globalThis` parameters, and dynamic
  imports or requires with no matching folder rule (`require-bindings`,
  `unconfigured`).
- A denied target in a condition the import does not select
  (`package_import_conditions_stop_at_null`). A selected `null` stops
  the runtime alias package-name check. The runtime file resolver can
  continue to a later condition and report its path. TypeScript type
  lookups also continue to later conditions.
- A wildcard `imports` key with an empty capture (such as `#x*end`
  against `#xend`) can select a different target from Node. The rule may
  also select legacy folder keys that Node skips.
- In an alias target array, an entry Node rejects as invalid (such as a
  `node:` URL target) is taken instead of falling through to the next entry.
- A `package.json` imports alias to a package subpath whose first
  segment is a Node built-in name, such as `path/custom`. The alias is
  taken as the built-in and skipped, while the same specifier imported
  directly is checked (review probe, follow-up).
- A `package.json` imports alias to an exported subpath of the current
  package's own name. The rule looks the name up only in `node_modules`,
  while Node reads the package's own `exports` (review probe, follow-up).
- A CommonJS `require` that only reaches a compiled `.node` addon
  through extension or index lookup (review probe, follow-up).
- The rule gives `package.json` imports precedence over a TypeScript
  `paths` entry for a full alias with a `?` suffix.
- When several `extends` entries disagree on `verbatimModuleSyntax`,
  the rule uses the first parent's value. TypeScript uses the last.
- A standalone loader lookup such as `require.resolve(name)` or
  `createRequire(base).resolve(name)`. It looks up a name and loads nothing
  (`require-bindings`, banned-imports `resolve-unknown-base` and
  `require_resolve_reports_only_when_the_result_reaches_a_loader` in
  `tests/boundary.rs`).
- Known false positive: destructuring `resolve` from a loader with an
  untrusted base reports “specifier is not a literal”
  (`destructuring_a_lookup_with_an_untrusted_base_reports_at_the_binding`).
- Calls made through `import { createRequire } from "#m"` when
  `package.json` maps `#m` to Node's `module` builtin. Loader
  bindings are classified before package aliases are resolved
  (probe, follow-up).
- `process` or `globalThis` exported from a module and used by an
  importer (`tracked_and_exported_process_and_global_values_stay_clean`).
- A standalone `Object.assign(globalThis, {...})` or
  `Object['assign'](globalThis, {...})` with the global `Object` and
  an object literal, including parentheses or TypeScript type wrappers
  around the literal
  (`global_filename_is_not_a_wrapper_base_in_module_packages`,
  `discarded_global_assign_accepts_static_methods_and_wrapped_literals`).
- Known false positive: `log += process` reports at `process`
  (`whole_process_and_global_values_report_at_the_use`).
- Type imports in these layouts can follow a different file than
  TypeScript 5.9 does, so a denied declaration can pass or an allowed
  one can report (review probes, follow-up):
  - a legacy package subpath folder with its own `package.json` `types`;
  - a written `.d.ts` or `.d.mts` subpath next to a matching `.ts` or
    `.mts` file;
  - a `paths` or `typesVersions` substitution that names an existing
    `.js` file;
  - a local folder whose `package.json` has `typesVersions`, or whose
    `typings` target is missing;
  - extensionless relative or `paths` targets imported from `.mts` or
    `.cts` files;
  - `node_modules/<name>.d.ts` beside a package folder of the same
    name, and in NodeNext import mode a package folder with an index
    but no `package.json`;
  - an `imports` alias to a bare package that has `typesVersions`;
  - a package importing itself by name while a package of the same
    name sits in `node_modules`.

## Known limits

With folder rules configured, these can still change an exit code or run
uncounted work:

- In `ci`, a corrupt blob that only this rule reads can stop the shared
  `git cat-file` batch, so `ci` exits 2.
- An invalid tracked symlink can fail the CI tree read before the rule
  reports (`link_directory_suffix_requires_a_directory`).
- A failed write of advisory output (for example stdout on a full disk)
  turns exit 0 into exit 2.
- TypeScript `paths` candidates are probed before the work budget is
  charged. Runtime file resolution also uses the resolver library's
  internal package export traversal.
- Wildcard substitution allocates the capture once per `*` in the target
  under a single charge.
- A long constant specifier is copied for each call that uses it.
- Reading and parsing a reachable re-export file happens before its
  import analysis is charged. Exhausting the per-source work budget
  reports an advisory and stops following that reading.
