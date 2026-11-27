# boundary/banned-imports

Advisory until the resolution cases under Does not catch are addressed. Findings print with `[advisory]` and leave the verdict unchanged.

## Catches

- A configured banned name or its slash subpaths in static imports,
  dynamic imports, re-exports, TypeScript import-equals and `require`
  calls. Builtins match with or without the `node:` prefix (`forms`).
- Type-only dependencies, including `import type`, `import()` type
  queries, `typeof import()` and type specifiers in mixed imports
  and re-exports (`forms`, `import-types`,
  `mixed_reexports_follow_the_exported_type_and_value_readings`).
  With `verbatimModuleSyntax` enabled, imports and re-exports containing
  only inline type specifiers also check the runtime target
  (`inline_type_specifiers_load_under_verbatim_module_syntax`).
- A banned package name or subpath behind a `package.json` imports
  alias. Exact and wildcard aliases match the full specifier, including
  any query or fragment. The applicable `import`, `require`,
  `node-addons` or `module-sync` condition can supply the name. Type-only
  aliases also try the selected declaration target
  (`package-aliases`, `node-addons`, `boundary-module-sync`,
  `type_package_imports_banned_rule_uses_resolved_condition`).
- Calls through aliases of `require`, `module.require`, loaders
  from `createRequire` and Node's `process.getBuiltinModule`
  (`require-bindings` and `closed_loader_forms` in
  `tests/boundary.rs`).
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
  “specifier is not a literal”. This includes results passed through a
  const alias and lookups made through aliased or destructured methods
  (`allowed_lookup_names_are_still_non_literal_load_arguments`,
  `dynamic_import_of_lookup_reports_non_literal`,
  `destructured_lookup_is_reported_when_loaded` and
  `runtime_builtins_loaded_through_resolve_report_at_the_load`).
- Dynamic load arguments, untrusted loader bases and loader escapes
  that make the specifier unknown. The rule also reports empty or
  unreadable specifiers, including file URLs and invalid percent escapes
  (`forms`, `require-base`, `require-bindings`, `empty-specifiers`,
  `lone-surrogate`, `url-specifiers`). Literal specifiers inside supported
  expression wrappers retain their positions
  (`wrapped-positions`, `non_null_wrapped_bases_and_specifiers_keep_their_findings`).
- Banned package subpaths with a query or fragment suffix in a static
  import (`banned_package_subpath_with_suffix` in `tests/boundary.rs`).
- In `blackhole ci`, banned dependencies in the head commit tree,
  including loads through a package imports alias or a loader lookup
  (`boundary-banned-package-alias`, `boundary-load-resolved-name`).

## Does not catch

- Allowed names that merely share a prefix with a banned one, such as
  `lodash-es` when `lodash` is banned, or shadowed loader parameters
  (`allowed`, `require-bindings`).
- Dynamic imports and requires when the banned list is empty
  (`unconfigured`).
- A banned target in a condition the import does not select
  (`package-aliases`, the clean `#unused` import). A selected `null` stops
  runtime conditional lookup. TypeScript type lookups continue to later
  conditions (`package_import_conditions_stop_at_null`).
- A wildcard `imports` key with an empty capture (such as `#x*end`
  against `#xend`) can select a different target from Node. The rule may
  also select legacy folder keys that Node skips.
- In an alias target array, an entry Node rejects as invalid (such as a
  `node:` URL target) is taken instead of falling through to the next entry.
- The rule gives `package.json` imports precedence over a TypeScript
  `paths` entry for a full alias with a `?` suffix.
- When several `extends` entries disagree on `verbatimModuleSyntax`,
  the rule uses the first parent's value. TypeScript uses the last.
- A standalone `require.resolve(name)` or
  `createRequire(base).resolve(name)`, even with an unknown base or
  dynamic name. A lookup loads nothing (`resolve-only`,
  `resolve-unknown-base`, `destructuring_a_lookup_does_not_load_anything` and
  `require_resolve_reports_only_when_the_result_reaches_a_loader` in
  `tests/boundary.rs`).
- Enumerating `require.cache` with `Object.keys` or
  `Object.getOwnPropertyNames` stays clean. Reading a cached module still
  reports (`require_cache_allows_only_key_enumeration`).
- Named and namespace calls to `getSourceMapsSupport` from an allowed
  `node:module` stay clean (`source_map_support_is_not_a_loader`).
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
- Loader calls in source strings passed to `eval` or `new Function`, and
  internal builtin names passed to `process.binding`
  (`loader_scope_controls_and_module_pattern_siblings`).
- Relative file imports that lead indirectly to a banned package. A
  direct dependency on the banned package is still checked in each
  scanned source file.
- Known false positive: `log += process` reports at `process`
  (`whole_process_and_global_values_report_at_the_use`).

## Known limits

With a ban list configured, these can still change an exit code or run
uncounted work:

- In `ci`, a corrupt blob that only this rule reads can stop the shared
  `git cat-file` batch, so `ci` exits 2.
- A failed write of advisory output (for example stdout on a full disk)
  turns exit 0 into exit 2.
- TypeScript `paths` candidates are probed before the work budget is
  charged.
- Wildcard substitution allocates the capture once per `*` in the target
  under a single charge.
- A long constant specifier is copied for each call that uses it, and
  each lookup compares against the whole ban list uncharged.
