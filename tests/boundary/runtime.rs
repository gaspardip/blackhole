use super::{UNKNOWN, assert_findings, judge, lines_for};

fn findings(path: &str, source: &str, expected: &[(usize, usize, &str)]) {
    assert_findings(path, source, expected, true, 0);
}

#[test]
fn exported_builtin_bindings() {
    findings(
        "a.cts",
        "export { getBuiltinModule as load } from 'node:process';\nexport {isBuiltin, builtinModules, type SomeType} from 'module';\nexport import m = require('node:module');",
        &[(1, 42, UNKNOWN), (3, 27, UNKNOWN)],
    );
}

#[test]
fn named_builtin_member_selected_at_import() {
    findings(
        "a.ts",
        "import { _load as load, createRequire, type Example } from 'module';\nload('allowed');",
        &[(1, 10, UNKNOWN)],
    );
}

#[test]
fn require_cache_allows_only_key_enumeration() {
    findings(
        "a.cjs",
        "Object.keys(require.cache);\nObject['getOwnPropertyNames']((require.cache));\nrequire.cache[path].exports;\nObject.values(require.cache);\nfunction inspect(Object) { Object.keys(require.cache); }",
        &[(3, 1, UNKNOWN), (4, 15, UNKNOWN), (5, 40, UNKNOWN)],
    );
}

#[test]
fn source_map_support_is_not_a_loader() {
    let out = judge(
        &[
            (
                "blackhole.toml",
                "[boundaries]\nbanned-imports=['restricted']",
            ),
            (
                "a.mjs",
                "import { getSourceMapsSupport } from 'node:module';\ngetSourceMapsSupport();",
            ),
            (
                "b.mjs",
                "import * as modules from 'node:module';\nmodules.getSourceMapsSupport();",
            ),
        ],
        &[],
        0,
    );
    assert!(out.is_empty(), "{out}");
}

#[test]
fn jsx_runtime_members() {
    findings(
        "a.tsx",
        "import * as p from 'node:process';\n<p.getBuiltinModule />;\n<globalThis.process.getBuiltinModule />;\n<globalThis.require></globalThis.require>;\n<module.filename />;\n<p.env />;",
        &[(2, 2, UNKNOWN), (3, 2, UNKNOWN), (4, 2, UNKNOWN)],
    );
}

#[test]
fn runtime_wrapper_bindings() {
    for decl in [
        "var require;",
        "var require; var require;",
        "declare const require: Function;",
    ] {
        findings(
            "a.cts",
            &format!("{decl}\nrequire('restricted');"),
            &[(2, 9, "restricted")],
        );
    }
    findings(
        "a.cts",
        "declare const process: Process;\nprocess.getBuiltinModule('restricted');\nfunction f(){var require; require('restricted');}",
        &[(2, 26, "restricted")],
    );
    findings(
        "a.cjs",
        "var arguments;\nconsume(arguments);\nvar module;\nmodule.require('restricted');",
        &[(2, 9, UNKNOWN), (4, 16, "restricted")],
    );
    findings("a.mjs", "var require; require('restricted');", &[]);
}

#[test]
fn untrusted_loader_members() {
    assert_findings(
        "a.cjs",
        "const load = require('module').createRequire('/other.cjs');\nconst resolve = load.resolve;\ntypeof load.main;\nconst {resolve:r} = load;\nload.length;",
        &[(2, 17, UNKNOWN), (3, 8, UNKNOWN), (4, 21, UNKNOWN)],
        true,
        1,
    );
}

#[test]
fn wrapped_factory_bases() {
    findings(
        "a.mts",
        "import {createRequire as f} from 'module';\nf(import.meta?.url)('allowed');\nf((0,import.meta).url)('allowed');\nf(import.meta[('url' as string)])('allowed');",
        &[],
    );
    findings(
        "a.cts",
        "require('module').createRequire((0,__filename))('restricted');",
        &[(1, 49, "restricted")],
    );
}

#[test]
fn meta_assignment_patterns() {
    findings(
        "a.mjs",
        "let url, rest;\n({url} = import.meta);\n({['url']:url} = import.meta);\n({url,...rest} = import.meta);\n({[getKey()]:url} = import.meta);",
        &[(4, 18, UNKNOWN), (5, 21, UNKNOWN)],
    );
}

#[test]
fn wrapper_root_read_positions() {
    assert_findings(
        "a.cjs",
        "typeof arguments;\nconst a = arguments; a === arguments;\nconst main = process.mainModule; main === module;\nconst {mainModule:m} = process; typeof m;\nconsume(m);\na[0];\nfunction f(){ return () => arguments[1]; }",
        &[(5, 9, UNKNOWN), (6, 1, UNKNOWN)],
        true,
        1,
    );
}

#[test]
fn loader_metadata_chains() {
    findings(
        "a.cjs",
        "require.name.toUpperCase(); module.require.length.toString();\nprocess.getBuiltinModule.name.trim();\nrequire('module').createRequire.name.toUpperCase();",
        &[],
    );
}

#[test]
fn only_eligible_patterns_bind_aliases() {
    for kind in ["let", "var"] {
        findings(
            "a.cjs",
            &format!("{kind} {{require: load}} = module;\nload('restricted');"),
            &[(1, 23, UNKNOWN)],
        );
    }
}

#[test]
fn sequence_tail_preserves_test_rule_positions() {
    let path = "a.test.ts";
    let out = judge(
        &[(
            path,
            "test('x', async () => { await (0, expect.poll(() => samples)).toHaveLength(1); });\ntest('y', () => { (0,expect(1)).toBe(2); });",
        )],
        &[],
        1,
    );
    assert_eq!(
        lines_for(&out, path),
        [
            "a.test.ts:1:35 ts/no-unsignaled-poll",
            "a.test.ts:2:19 ts/no-assertionless-tests",
            "a.test.ts:2:22 ts/no-assertionless-tests"
        ]
    );
}

#[test]
fn whole_process_and_global_values_report_at_the_use() {
    findings(
        "a.cjs",
        "let p;\np = process ?? {};\np.getBuiltinModule('restricted');\nlet g = globalThis ?? {};\ng.require('restricted');\nlet log = ''; log += process;\nlet p2 = process;\nvar g2 = globalThis;\nconst root = globalThis; consume(root);\nconst state = cond ? process : x;\nconst spread = {...process};\nconst array = [globalThis];\nconst obj = {process};\nfunction get() { return process; }\nconst copy = Object.assign(globalThis, {});\nfunction patch(Object) { Object.assign(globalThis, {}); }",
        &[
            (2, 5, UNKNOWN),
            (4, 9, UNKNOWN),
            (6, 22, UNKNOWN),
            (7, 10, UNKNOWN),
            (8, 10, UNKNOWN),
            (9, 34, UNKNOWN),
            (10, 22, UNKNOWN),
            (11, 20, UNKNOWN),
            (12, 16, UNKNOWN),
            (13, 14, UNKNOWN),
            (14, 25, UNKNOWN),
            (15, 28, UNKNOWN),
            (16, 40, UNKNOWN),
        ],
    );
}

#[test]
fn discarded_process_and_global_values_only_check_dependencies() {
    let loads =
        "require('node:process');\nawait import('node:process');\nvoid require('node:process');";
    findings(
        "a.mts",
        &format!("{loads}\nconst p = require('node:process'); f(p);\nprocess;\nvoid globalThis;"),
        &[(4, 38, UNKNOWN)],
    );
    let out = judge(
        &[
            (
                "blackhole.toml",
                "[boundaries]\nbanned-imports=['node:process']",
            ),
            ("a.mts", loads),
        ],
        &[],
        0,
    );
    assert_eq!(
        lines_for(&out, "a.mts:"),
        [
            "a.mts:1:9 boundary/banned-imports [advisory] node:process",
            "a.mts:2:14 boundary/banned-imports [advisory] node:process",
            "a.mts:3:14 boundary/banned-imports [advisory] node:process",
        ],
    );
}

#[test]
fn discarded_global_assign_accepts_static_methods_and_wrapped_literals() {
    assert_findings(
        "a.mts",
        "Object.assign(globalThis, ({}));\nObject['assign'](globalThis, {});\nObject.assign(globalThis, {} as object);\nObject.assign(globalThis, obj);\nfunction patch(Object) { Object.assign(globalThis, {}); }\nconst result = Object['assign'](globalThis, ({}));\nObject[method](globalThis, {});",
        &[
            (4, 15, UNKNOWN),
            (5, 40, UNKNOWN),
            (6, 33, UNKNOWN),
            (7, 16, UNKNOWN),
        ],
        true,
        1,
    );
}

#[test]
fn tracked_and_exported_process_and_global_values_stay_clean() {
    findings(
        "a.mts",
        "const p = process; const q = p; q.env;\nconst g = globalThis; g['require']('allowed');\nexport {p};\nexport default g;\nfunction read(process, globalThis) { return [process, globalThis]; }",
        &[],
    );
    findings("a.cts", "const p = process; export = p;", &[]);
}

#[test]
fn type_package_imports_banned_rule_uses_resolved_condition() {
    let out = judge(
        &[
            ("blackhole.toml", "[boundaries]\nbanned-imports=['demo']"),
            (
                "ui/package.json",
                r##"{"imports":{"#model":{"types":"missing-package","require":"demo"}}}"##,
            ),
            ("ui/app.cts", "import type {Model} from '#model';"),
            ("node_modules/demo/package.json", r#"{"name":"demo"}"#),
            (
                "node_modules/demo/index.d.ts",
                "export type Model = string;",
            ),
        ],
        &[],
        0,
    );
    assert_eq!(
        lines_for(&out, "ui/app.cts"),
        ["ui/app.cts:1:26 boundary/banned-imports [advisory] demo"]
    );
}

#[test]
fn mixed_reexports_follow_the_exported_type_and_value_readings() {
    let source = "import { live, type Model } from '#dep'; export { live, type Model };\nexport { live, type Model } from '#dep';";
    for banned in ["types-dep", "runtime-dep"] {
        let out = judge(
            &[
                (
                    "blackhole.toml",
                    &format!("[boundaries]\nbanned-imports=['{banned}']"),
                ),
                (
                    "package.json",
                    r##"{"imports":{"#dep":{"types":"types-dep","import":"runtime-dep"}}}"##,
                ),
                (
                    "node_modules/types-dep/package.json",
                    r#"{"types":"index.d.ts"}"#,
                ),
                (
                    "node_modules/types-dep/index.d.ts",
                    "export type Model = string;",
                ),
                ("app.mts", source),
            ],
            &[],
            0,
        );
        for (line, text) in source.lines().enumerate() {
            assert!(
                out.contains(&format!(
                    "app.mts:{}:{} boundary/banned-imports [advisory] {banned}",
                    line + 1,
                    text.find("'#dep'").unwrap() + 1
                )),
                "{out}"
            );
        }
    }
}

#[test]
fn non_null_wrapped_bases_and_specifiers_keep_their_findings() {
    let source = concat!(
        "import {createRequire} from 'module';\n",
        "createRequire((__filename)!)('allowed');\n",
        "require((('restricted' satisfies string)!));\n",
        "createRequire(import.meta.url)((`restricted`)!);",
    );
    assert_findings(
        "ui/a.ts",
        source,
        &[
            (2, 30, UNKNOWN),
            (3, 11, "restricted"),
            (4, 33, "restricted"),
        ],
        true,
        1,
    );
}

#[test]
fn global_filename_write_reports_at_the_assignment() {
    findings(
        "a.mjs",
        "import {createRequire as f} from 'module';\n__filename = 'elsewhere';\nf(import.meta.url)('./x.cjs');",
        &[(2, 1, UNKNOWN)],
    );
}

#[test]
fn node_module_source_map_queries_do_not_load_code() {
    let source = "import {findSourceMap, register} from 'node:module';\n\
                  import * as modules from 'node:module';\n\
                  findSourceMap(name);\n\
                  modules.findSourceMap(name);\n\
                  register(name);";
    findings("app.mjs", source, &[(1, 24, UNKNOWN)]);
}
