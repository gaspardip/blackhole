# ts/no-conditional-empty-object-spread

## Catches

- Object spreads of a ternary with an empty object in either branch
  (`invalid-01.ts`, `invalid-02.ts`).
- Nested conditionals and branches or spread operands with TypeScript
  wrappers (`nested-conditional.ts`, `upstream-accept-wrapped-branches.ts`).
- Empty local consts and const alias chains whose uses are confined to
  harmless destinations, whatever type annotation they carry
  (`alias-of-alias.ts`, `upstream-accept-visible-spread-alias.ts`,
  `annotated-alias.ts`).
- Harmless uses through `void`, unused aliases and type-only re-exports
  from another module (`void-read.ts`, `unused-alias.ts`,
  `type-reexport.ts`).

## Does not catch

- A member of an object that is written, deleted, passed, aliased,
  method-called or named in a `typeof` type query anywhere in the file.
  Member values are trusted only for untouched const objects (follow-up).
- An empty value observed outside a spread or discarded read, including
  an escaped argument, an exported alias or a type query. Visibility is
  part of the rule (`escaped-empty-value.ts`,
  `observable-alias-default.ts`, `observable-alias-query.ts`).
- A conditional used outside an object spread. Its arms do not end at a
  supported destination (`non-object-destinations.ts`).
- An alias cycle. The rule cannot establish a harmless empty object (`alias-cycle.ts`).
- An empty const or alias read anywhere above the end of its declaration, even
  inside a function or class body that runs later. The rule does not decide when
  such code runs (`before-initialization.ts`, `hoisted-reader.ts`,
  `field-before-declaration.ts`, `immediate-constructor.ts`, `immediate-tag.ts`).
- An empty value reached through a member such as `p.empty`. The visibility
  check follows local const bindings and has no member projection (probe).
- An empty object also read in a comma expression,
  such as `(empty, log())`
  (`documented_slop_rule_limits` in `tests/spellings.rs`).
- An empty const reached through an identifier inside a `with` body, or
  anywhere in a file that mentions anything named `eval`, since either can
  replace or fill the object. An empty object literal written in the
  conditional still counts
  (`conditional_empty_spreads_hide_aliases_in_dynamic_scope` in
  `tests/spellings.rs`).
- A value that shares its name with an interface or type alias. The rule
  skips it rather than guess which declaration the name means
  (`documented_slop_rule_limits` in `tests/spellings.rs`).
