use blackhole_check::engine::{registry, run};

const CASES: &[(&str, &str, usize)] = &[
    (
        "import * as t from 'node:test';\nt.test.skip(\"x\",fn);",
        "ts/no-skipped-tests",
        8,
    ),
    (
        "import * as t from 'node:test';\nt.test.only(\"x\",fn);",
        "ts/no-skipped-tests",
        8,
    ),
    (
        "import * as t from 'node:test';\nt.test.todo(\"x\",fn);",
        "ts/no-skipped-tests",
        8,
    ),
    (
        "import * as t from 'node:test';\nt.test(\"x\",{skip:true},fn);",
        "ts/no-skipped-tests",
        13,
    ),
    (
        "import * as t from 'node:test';\nt.describe.skip(\"x\",fn);",
        "ts/no-skipped-tests",
        12,
    ),
    (
        "import * as t from 'node:test';\nt.describe.only(\"x\",fn);",
        "ts/no-skipped-tests",
        12,
    ),
    (
        "import * as t from 'node:test';\nt.describe.todo(\"x\",fn);",
        "ts/no-skipped-tests",
        12,
    ),
    (
        "import * as t from 'node:test';\nt.describe(\"x\",{skip:true},fn);",
        "ts/no-skipped-tests",
        17,
    ),
    (
        "import * as t from 'node:test';\nt.it.skip(\"x\",fn);",
        "ts/no-skipped-tests",
        6,
    ),
    (
        "import * as t from 'node:test';\nt.it.only(\"x\",fn);",
        "ts/no-skipped-tests",
        6,
    ),
    (
        "import * as t from 'node:test';\nt.it.todo(\"x\",fn);",
        "ts/no-skipped-tests",
        6,
    ),
    (
        "import * as t from 'node:test';\nt.it(\"x\",{skip:true},fn);",
        "ts/no-skipped-tests",
        11,
    ),
    (
        "import * as t from 'node:test';\nt.default.skip(\"x\",fn);",
        "ts/no-skipped-tests",
        11,
    ),
    (
        "import * as t from 'node:test';\nt.default.only(\"x\",fn);",
        "ts/no-skipped-tests",
        11,
    ),
    (
        "import * as t from 'node:test';\nt.default.todo(\"x\",fn);",
        "ts/no-skipped-tests",
        11,
    ),
    (
        "import * as t from 'node:test';\nt.default(\"x\",{skip:true},fn);",
        "ts/no-skipped-tests",
        16,
    ),
    (
        "import * as t from 'node:test';\nt.test(\"x\",async()=>{await Bun.sleep(1);assert(ok())});",
        "ts/no-fixed-sleep",
        22,
    ),
    (
        "import * as t from 'node:test';\nt.test(\"x\",()=>{});",
        "ts/no-assertionless-tests",
        1,
    ),
    (
        "import * as t from 'node:test';\nt.test(\"x\",{retry:2},fn);",
        "ts/no-flake-masking",
        13,
    ),
    (
        "import * as t from '@playwright/test';\nt.test.skip(\"x\",fn);",
        "ts/no-skipped-tests",
        8,
    ),
    (
        "import * as t from '@playwright/test';\nt.test.only(\"x\",fn);",
        "ts/no-skipped-tests",
        8,
    ),
    (
        "import * as t from '@playwright/test';\nt.test.todo(\"x\",fn);",
        "ts/no-skipped-tests",
        8,
    ),
    (
        "import * as t from '@playwright/test';\nt.test(\"x\",{skip:true},fn);",
        "ts/no-skipped-tests",
        13,
    ),
    (
        "import * as t from '@playwright/test';\nt.describe.skip(\"x\",fn);",
        "ts/no-skipped-tests",
        12,
    ),
    (
        "import * as t from '@playwright/test';\nt.describe.only(\"x\",fn);",
        "ts/no-skipped-tests",
        12,
    ),
    (
        "import * as t from '@playwright/test';\nt.describe.todo(\"x\",fn);",
        "ts/no-skipped-tests",
        12,
    ),
    (
        "import * as t from '@playwright/test';\nt.describe(\"x\",{skip:true},fn);",
        "ts/no-skipped-tests",
        17,
    ),
    (
        "import * as t from '@playwright/test';\nt.default.skip(\"x\",fn);",
        "ts/no-skipped-tests",
        11,
    ),
    (
        "import * as t from '@playwright/test';\nt.default.only(\"x\",fn);",
        "ts/no-skipped-tests",
        11,
    ),
    (
        "import * as t from '@playwright/test';\nt.default.todo(\"x\",fn);",
        "ts/no-skipped-tests",
        11,
    ),
    (
        "import * as t from '@playwright/test';\nt.default(\"x\",{skip:true},fn);",
        "ts/no-skipped-tests",
        16,
    ),
    (
        "import * as t from '@playwright/test';\nt.test(\"x\",async()=>{await Bun.sleep(1);assert(ok())});",
        "ts/no-fixed-sleep",
        22,
    ),
    (
        "import * as t from '@playwright/test';\nt.test(\"x\",()=>{});",
        "ts/no-assertionless-tests",
        1,
    ),
    (
        "import * as t from '@playwright/test';\nt.test(\"x\",{retry:2},fn);",
        "ts/no-flake-masking",
        13,
    ),
    (
        "import * as t from 'vitest';\nt.test.skip(\"x\",fn);",
        "ts/no-skipped-tests",
        8,
    ),
    (
        "import * as t from 'vitest';\nt.test.only(\"x\",fn);",
        "ts/no-skipped-tests",
        8,
    ),
    (
        "import * as t from 'vitest';\nt.test.todo(\"x\",fn);",
        "ts/no-skipped-tests",
        8,
    ),
    (
        "import * as t from 'vitest';\nt.test(\"x\",{skip:true},fn);",
        "ts/no-skipped-tests",
        13,
    ),
    (
        "import * as t from 'vitest';\nt.describe.skip(\"x\",fn);",
        "ts/no-skipped-tests",
        12,
    ),
    (
        "import * as t from 'vitest';\nt.describe.only(\"x\",fn);",
        "ts/no-skipped-tests",
        12,
    ),
    (
        "import * as t from 'vitest';\nt.describe.todo(\"x\",fn);",
        "ts/no-skipped-tests",
        12,
    ),
    (
        "import * as t from 'vitest';\nt.describe(\"x\",{skip:true},fn);",
        "ts/no-skipped-tests",
        17,
    ),
    (
        "import * as t from 'vitest';\nt.it.skip(\"x\",fn);",
        "ts/no-skipped-tests",
        6,
    ),
    (
        "import * as t from 'vitest';\nt.it.only(\"x\",fn);",
        "ts/no-skipped-tests",
        6,
    ),
    (
        "import * as t from 'vitest';\nt.it.todo(\"x\",fn);",
        "ts/no-skipped-tests",
        6,
    ),
    (
        "import * as t from 'vitest';\nt.it(\"x\",{skip:true},fn);",
        "ts/no-skipped-tests",
        11,
    ),
    (
        "import * as t from 'vitest';\nt.test(\"x\",async()=>{await Bun.sleep(1);assert(ok())});",
        "ts/no-fixed-sleep",
        22,
    ),
    (
        "import * as t from 'vitest';\nt.test(\"x\",()=>{});",
        "ts/no-assertionless-tests",
        1,
    ),
    (
        "import * as t from 'vitest';\nt.test(\"x\",{retry:2},fn);",
        "ts/no-flake-masking",
        13,
    ),
    (
        "import * as t from '@jest/globals';\nt.test.skip(\"x\",fn);",
        "ts/no-skipped-tests",
        8,
    ),
    (
        "import * as t from '@jest/globals';\nt.test.only(\"x\",fn);",
        "ts/no-skipped-tests",
        8,
    ),
    (
        "import * as t from '@jest/globals';\nt.test.todo(\"x\",fn);",
        "ts/no-skipped-tests",
        8,
    ),
    (
        "import * as t from '@jest/globals';\nt.test(\"x\",{skip:true},fn);",
        "ts/no-skipped-tests",
        13,
    ),
    (
        "import * as t from '@jest/globals';\nt.describe.skip(\"x\",fn);",
        "ts/no-skipped-tests",
        12,
    ),
    (
        "import * as t from '@jest/globals';\nt.describe.only(\"x\",fn);",
        "ts/no-skipped-tests",
        12,
    ),
    (
        "import * as t from '@jest/globals';\nt.describe.todo(\"x\",fn);",
        "ts/no-skipped-tests",
        12,
    ),
    (
        "import * as t from '@jest/globals';\nt.describe(\"x\",{skip:true},fn);",
        "ts/no-skipped-tests",
        17,
    ),
    (
        "import * as t from '@jest/globals';\nt.it.skip(\"x\",fn);",
        "ts/no-skipped-tests",
        6,
    ),
    (
        "import * as t from '@jest/globals';\nt.it.only(\"x\",fn);",
        "ts/no-skipped-tests",
        6,
    ),
    (
        "import * as t from '@jest/globals';\nt.it.todo(\"x\",fn);",
        "ts/no-skipped-tests",
        6,
    ),
    (
        "import * as t from '@jest/globals';\nt.it(\"x\",{skip:true},fn);",
        "ts/no-skipped-tests",
        11,
    ),
    (
        "import * as t from '@jest/globals';\nt.test(\"x\",async()=>{await Bun.sleep(1);assert(ok())});",
        "ts/no-fixed-sleep",
        22,
    ),
    (
        "import * as t from '@jest/globals';\nt.test(\"x\",()=>{});",
        "ts/no-assertionless-tests",
        1,
    ),
    (
        "import * as t from '@jest/globals';\nt.test(\"x\",{retry:2},fn);",
        "ts/no-flake-masking",
        13,
    ),
    (
        "\nrequire('node:test').default.skip(\"x\",fn);",
        "ts/no-skipped-tests",
        30,
    ),
    (
        "const m=require('node:test');\nm.default.skip(\"x\",fn);",
        "ts/no-skipped-tests",
        11,
    ),
    (
        "import {default as t} from 'node:test';\nt.skip(\"x\",fn);",
        "ts/no-skipped-tests",
        3,
    ),
    (
        "const {default:t}=require('node:test');\nt.skip(\"x\",fn);",
        "ts/no-skipped-tests",
        3,
    ),
    (
        "import * as t from 'node:test';const u=t;\nu.default.skip(\"x\",fn);",
        "ts/no-skipped-tests",
        11,
    ),
    (
        "import * as t from 'msw/node';\nt.setupServer().listen();",
        "ts/strict-msw",
        1,
    ),
    (
        "import * as t from 'msw/browser';\nt.setupWorker().start();",
        "ts/strict-msw",
        1,
    ),
    (
        "import * as t from 'vitest';\ntest('x',()=>{t.expect(1).toBe(1)});",
        "ts/no-assertionless-tests",
        15,
    ),
    (
        "import * as t from '@jest/globals';\ntest('x',()=>{t.expect(1).toBe(1)});",
        "ts/no-assertionless-tests",
        15,
    ),
    (
        "import * as t from '@playwright/test';\ntest('x',()=>{t.expect(1).toBe(1)});",
        "ts/no-assertionless-tests",
        15,
    ),
];

#[test]
fn namespace_spellings() {
    for &(source, rule, col) in CASES {
        let found = run(registry(), [("case.test.ts", source.as_bytes())]);
        assert_eq!(
            found
                .iter()
                .map(|f| (f.rule, f.line, f.col))
                .collect::<Vec<_>>(),
            [(rule, 2, col)],
            "{source}"
        );
    }
}
