# ts/no-shape-in-symbol-names

## Catches

This list is complete: the rule reports these shapes and nothing else.

A case-insensitive `shape` inside one of these names:

1. A binding (including the name a shorthand pattern such as
   `const { shape } = v` binds), an identifier reference, the root of a JSX
   element name, a type name, a label or a private name (`invalid-01.ts`
   through `invalid-05.ts`, `wrapped-symbol.ts`, `loop-labels.ts`).
2. A property, method or accessor name written as an identifier or a
   string literal in an object literal, class, interface or type literal,
   and an enum member name written as an identifier (`invalid-01.ts`
   through `invalid-05.ts`, `quoted-property-keys.ts`, `enum-members.ts`).
3. A quoted enum member, import or export specifier name, `export * as`
   name, or import attribute key (`quoted-symbol-names.ts`,
   `name-literals.ts`).
4. A computed property or class member key whose expression is a known
   string (`upstream-accept-owned-keys.ts`, `concatenated-keys.ts`,
   `name-literals.ts`, `wrapped-computed-keys.ts`,
   `sequence-and-optional-keys.ts`).

A known string is one of:

- a string literal, or a template literal with no substitutions;
- `a + b` where `a` and `b` are both known strings, within the size budget;
- a reference to a `const` declared with a plain name (not destructured)
  whose initializer is a known string
  (`wrapped_computed_names_fold_neutral_aliases` in `tests/spellings.rs`);
- `o.name`, `o["name"]` or `o[0]`, optional (`o?.name`) or not, reading a
  field of an untouched const object whose value is a known string. The
  selector is a string literal, a template literal with no substitutions,
  a reference to a plain-named `const` whose initializer is one of these,
  or a plain non-negative decimal integer below 2^53, any of them wrapped
  as below (`untouched_const_owners_supply_member_values` in
  `tests/spellings.rs`);
- any of these in parentheses, behind `as`, `!`, `satisfies` or `<T>`, or
  as the last expression of a comma sequence such as `(f(), 'userShape')`.

An untouched const object is `const o = { ... }` with a plain name, where:

- every property is `key: value` (shorthand `{ key }` included) or a
  method, with no spread, getter, setter or `__proto__` key;
- every key is a name, a string literal or a template literal with no
  substitutions (bracketed or not), or a plain non-negative decimal
  integer below 2^53 written without brackets;
- the object is not exported at runtime (`export type { o }` does not
  count);
- every other use of `o` reads a field: `o.x`, `o[...]` or `<o.X />`, never written,
  deleted, passed, aliased, method-called or named in a `typeof` query.

## Does not catch

Anything outside the list above. In particular:

- A JSX attribute name such as `<area shape="rect" />`, any part of a
  namespaced JSX name such as `<svg:shape />` or `xlink:shape`
  (`jsx-attribute-names.tsx`, `jsx-namespaced-names.tsx`), a member property
  name such as `a.shape` or `<a.Shape />`, a destructuring or assignment
  key that is not also the bound name, such as `{ shape: s }`, and a name
  after the root of a `typeof` query. These name things the code does not
  own (`name-literal-values.ts`, `assignment-external-keys.ts`,
  `destructured-external-keys.ts`).
- String values that are not names.
- A const alias that is destructured, such as
  `const { key } = { key: 'userShape' }`.
- An owner key computed from anything but a string or template literal,
  such as `{ [field]: 'userShape' }` or `{ [0]: 'userShape' }`; that
  object is not untouched.
- An addition with a non-string operand, such as `'userShape' + 1`.
- An interpolated template such as `` `sha${'pe'}` ``, a call such as
  `'sha'.concat('pe')`, and an enum member value such as `K.A` (probes).
- A selector built by addition or read from a field, such as
  `names[prefix + '-']` or `parts[keys.field]`.
- Cyclic, dynamic or over-budget additions
  (`cyclic_shared_additions_share_failed_folds`,
  `repeated_cyclic_key_prefixes_share_failed_folds`,
  `repeated_dynamic_key_prefixes_share_failed_folds` and
  `long_computed_key_chains_share_fold_results` in
  `tests/resolution_time.rs`, and
  `concatenation_budget_is_independent_of_key_order` and
  `unknown_member_values_do_not_depend_on_read_order` in
  `tests/spellings.rs`).
- Any const or field value inside a `with` block, or anywhere in a file
  that mentions anything named `eval`, including type positions
  (`direct_eval_hides_member_and_const_string_values`,
  `every_eval_mention_hides_member_and_const_string_values`,
  `with_bodies_hide_member_and_const_string_values` and
  `accessor_prototype_and_spread_owners_are_unknown` in
  `tests/spellings.rs`).
- Numeric keys other than plain non-negative decimal integers below 2^53
  (`only_plain_safe_integer_keys_supply_numeric_members` in
  `tests/spellings.rs`).
- A const object or name shared with a type declaration
  (`computed_names_skip_values_shared_with_types` in `tests/spellings.rs`).
- Known false positives: code run through `Function`, `new Function`, a
  timer string, or a JSX string that spells `eval` with character
  references such as `ev&#x61;l` can change field values in a classic
  script without mentioning `eval`; folding still trusts those values
  (`every_eval_mention_hides_member_and_const_string_values` and
  `jsx_character_references_spelling_eval_keep_values` in
  `tests/spellings.rs`).
