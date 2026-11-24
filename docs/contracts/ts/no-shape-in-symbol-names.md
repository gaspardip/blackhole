# ts/no-shape-in-symbol-names

## Catches

- Case-insensitive `shape` inside bindings, identifier references, type
  names, property signatures and labels (`invalid-01.ts` through
  `invalid-05.ts`, `wrapped-symbol.ts`, `loop-labels.ts`).
- Quoted enum members, import and export specifier names, and import
  attribute keys (`quoted-symbol-names.ts`, `name-literals.ts`).
- Owned keys written as string or template literals and keys that fold
  from string addition (`upstream-accept-owned-keys.ts`,
  `concatenated-keys.ts`, `name-literals.ts`).
- Local aliases and TypeScript wrappers in computed property or class
  names (`wrapped-computed-keys.ts` and
  `wrapped_computed_names_fold_neutral_aliases` in `tests/spellings.rs`).
- A string property read from an untouched const object in a computed
  name (`untouched_const_owners_supply_member_values` in
  `tests/spellings.rs`).

## Does not catch

- Member values and const string values inside a `with` block or anywhere
  in a file that mentions anything named `eval`, including type positions.
  Member values on an object literal with accessors, a `__proto__` key or a
  spread are also unknown
  (`direct_eval_hides_member_and_const_string_values`,
  `every_eval_mention_hides_member_and_const_string_values`,
  `with_bodies_hide_member_and_const_string_values` and
  `accessor_prototype_and_spread_owners_are_unknown` in `tests/spellings.rs`).
- Code run through `Function`, `new Function` or a string passed to a
  timer can change member values in a classic script without mentioning
  `eval`. Folding still trusts those values
  (`every_eval_mention_hides_member_and_const_string_values` in
  `tests/spellings.rs`).
- A JSX string that spells `eval` with character
  references, such as `ev&#x61;l`, does not count as
  a mention. Reaching `eval` from it needs a runtime
  key, the same gap as `Function` and timer strings
  (`jsx_character_references_spelling_eval_keep_values` in
  `tests/spellings.rs`).
- Member values on an object literal with numeric keys other than
  plain non-negative decimal integers below 2^53
  (`only_plain_safe_integer_keys_supply_numeric_members` in `tests/spellings.rs`).
- A member of an object that is written, deleted, passed, aliased,
  method-called or named in a `typeof` type query anywhere in the file.
  Member values are trusted only for untouched const objects (follow-up).
- String values and external member or destructuring keys. These uses
  are outside the owned-name check (`name-literal-values.ts`,
  `assignment-external-keys.ts`, `destructured-external-keys.ts`).
- An interpolated template such as `` `sha${'pe'}` ``. String folding supports
  templates without substitutions (probe).
- An enum member value such as `K.A` from `enum K { A = 'shape' }`.
  Member projection does not evaluate enum initializers
  (probe).
- A call such as `'sha'.concat('pe')`. Calls are outside string folding
  (probe).
- A computed name from `parts[keys.field]`. Folding a member read
  used as a member selector is a follow-up (probe).
- Cyclic, dynamic or over-budget additions. These cannot supply a known
  key (`cyclic_shared_additions_share_failed_folds`,
  `repeated_cyclic_key_prefixes_share_failed_folds`,
  `repeated_dynamic_key_prefixes_share_failed_folds` and
  `long_computed_key_chains_share_fold_results` in
  `tests/resolution_time.rs`, and
  `concatenation_budget_is_independent_of_key_order` and
  `unknown_member_values_do_not_depend_on_read_order` in
  `tests/spellings.rs`).
- A string-property write after `await` or an unreachable write after
  `return` hides a known key (probes).
- An assignment to a property with an accessor or a write through an
  owner passed to a call can leave a computed name unknown
  (`accessor_assignments_do_not_supply_member_values` in
  `tests/spellings.rs` and `repeated_member_reads_do_not_rescan_other_effects`
  in `tests/resolution_time.rs`).
- A computed name from a const whose name is shared by a type declaration
  (`computed_names_skip_values_shared_with_types` in `tests/spellings.rs`).
