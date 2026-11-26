# ts/no-reduce-accumulator-copy

Advisory: findings print with `[advisory]` and never make a verdict red. Findings stayed open after two review rounds, so by the project's review rule it does not gate.

## Catches

This list is complete: the rule reports these shapes and nothing else.

The receiver must be proven to be an array.

In a `reduce` or `reduceRight` callback, written inline or as a declared,
overloaded or aliased function (`declared-reducer.ts`,
`overloaded-reducer.ts`), one of these copies of the incoming accumulator,
the callback's first parameter `acc`:

1. `{ ...acc }` or `[...acc]` in an object or array literal (`valid-18.ts`,
   `valid-19.ts`).
2. `Object.assign(target, ..., acc, ...)` with no spread argument before
   `acc`, where `target` is a `{}` or `[]` literal, an untouched const
   member, or a local whose own `let`, `const` or `var` declarator holds
   that literal, that nothing else writes, read later in the same function
   inside the declaration's block (`invalid-01.ts`, `invalid-08.ts`,
   `fresh-array-target.ts`, `local-assign-target.ts`,
   `computed-fresh-target.ts`).
3. `Array.from(acc)`, or `acc.concat`, `slice`, `toSpliced`, `toSorted`,
   `toReversed` or `with`, when the seed or the parameter's annotation
   proves an array (`invalid-11.ts` through `invalid-20.ts`).

The copy:

- is in the callback's body, or in a function called where it is written
  inside it, through `.call`, `Reflect.apply` with a literal argument array
  or an immediately called `.bind`, unless that function is a generator
  (`immediate-reducer-copy.ts`, `immediate-invocation-wrappers.ts`);
- has no `await`, `yield`, `for await` or `await using` written between
  the start of the callback and the copy, including arguments evaluated
  before the copying call runs;
- reads the incoming value: `acc` is written only by plain `acc = ...`
  statements in the callback's own body, outside any loop, and the copy
  comes before the first of them or inside its right-hand side.

Method spellings include dot access, string and template keys, readable
const keys and method aliases. Optional calls, parentheses and TypeScript
assertions are transparent (`invalid-02.ts`, `invalid-06.ts`,
`non-null-optional-chain.ts`, `member-reducer-receiver.ts`).

## Does not catch

Anything outside the list above. In particular:

- A callback that writes `acc` any other way: destructuring, `++`,
  `for (acc of ...)`, a target wrapped in `as` or `!`, or a write in a
  nested function or a loop. The rule reports nothing in that callback.
- An immediate call through a spread, such as `Reflect.apply(...[() => [...acc], null, []])`.
- An `Object.assign` source after a spread argument, such as
  `Object.assign({}, ...layers, acc)`.
- Copies in deferred callbacks, generators, functions run with `new`, or
  callees picked by `?:`, `??`, `||` or `&&` (`deferred-reducer-copy.ts`,
  `documented_reducer_limits` in `tests/spellings.rs`).
- Targets that are shared, conditionally or later assigned, aliased,
  written by a nested callback, not `const` beside a call through
  anything named `eval`, or a `var` sharing a name with a block function
  (existing fixtures and tests).
- Anything in a file with a `with` statement; member and const string
  values in a file that mentions anything named `eval`; a name shared with
  an interface or type alias (existing tests).
- After a fixed amount of work per file, the rule reports nothing more there.
- `Object.assign({}, acc, { async f() { await ready; } })`: a suspension
  in a function created but not run before the copy still silences it.
- A fresh member under a computed `__proto__` key, such as
  `{ ['__proto__']: {} }`.
- Known false positives: a detached call such as
  `(0, xs.reduce)(acc => ({ ...acc }), {})`; a direct `eval` string
  that rebinds `acc`; an array whose `reduce` or copy method was replaced
  (`rows.reduce = (_step, seed) => seed`, `acc.slice = () => acc`); and
  `Reflect.apply` whose argument array arrives through a spread, such as
  `Reflect.apply(() => [...acc], ...[null, null], [])`.
