#[path = "common/cli.rs"]
mod cli;
mod common;
use std::{fs, os::unix::fs::symlink, path::Path};

fn put(root: &Path, name: &str, content: &str) {
    let path = root.join(name);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, content).unwrap();
}

fn judge(files: &[(&str, &str)], links: &[(&str, &str)], code: i32) -> String {
    let dir = cli::repo();
    let root = dir.path();
    for &(name, text) in files {
        put(root, name, text);
    }
    for &(name, target) in links {
        symlink(target, root.join(name)).unwrap();
    }
    common::git(root, &["add", "-f", "."]).unwrap();
    common::git(root, &["commit", "-qm", "case"]).unwrap();
    let outputs = [
        cli::check(root, &[], code),
        cli::run(root, &["ci", "--base", "HEAD", "--head", "HEAD"], code),
    ]
    .map(|output| {
        let text = String::from_utf8(if code == 2 {
            output.stderr
        } else {
            output.stdout
        })
        .unwrap();
        text.trim_end_matches("blackhole ci: red (findings)\n")
            .to_owned()
    });
    assert_eq!(outputs[0], outputs[1]);
    outputs.into_iter().next().unwrap()
}

fn lines_for<'a>(out: &'a str, prefix: &str) -> Vec<&'a str> {
    out.lines()
        .filter(|line| line.starts_with(prefix))
        .collect()
}

const UNKNOWN: &str = "specifier is not a literal";

fn findings(path: &str, source: &str, expected: &[(usize, usize, &str)]) {
    assert_findings(path, source, expected, false, 0);
}

fn assert_findings(
    path: &str,
    source: &str,
    expected: &[(usize, usize, &str)],
    boundary_only: bool,
    code: i32,
) {
    let out = judge_banned(path, source, code);
    let lines: Vec<_> = expected
        .iter()
        .map(|(line, col, note)| {
            format!("{path}:{line}:{col} boundary/banned-imports [advisory] {note}")
        })
        .collect();
    let boundary: Vec<_> = lines_for(&out, path)
        .into_iter()
        .filter(|line| !boundary_only || line.contains(" boundary/"))
        .collect();
    assert_eq!(boundary, lines, "{source}");
    if !boundary_only && expected.is_empty() {
        assert!(out.is_empty(), "{out}");
    }
}

#[test]
fn banned_package_subpath_with_suffix() {
    for suffix in ["?raw", "#raw"] {
        let out = judge(
            &[
                (
                    "blackhole.toml",
                    "[boundaries]\nbanned-imports=['pkg/x.mjs']",
                ),
                ("ui/a.mjs", &format!("import 'pkg/x.mjs{suffix}';")),
                ("node_modules/pkg/package.json", r#"{"name":"pkg"}"#),
                ("node_modules/pkg/x.mjs", ""),
            ],
            &[],
            0,
        );
        assert!(out.contains("ui/a.mjs:1:8 boundary/banned-imports [advisory] pkg/x.mjs"));
    }
}

#[test]
fn package_import_keys_keep_query_suffixes() {
    let out = judge(
        &[
            (
                "blackhole.toml",
                "[boundaries]\nbanned-imports=['restricted']",
            ),
            (
                "package.json",
                r##"{"imports":{"#banned":"allowed","#banned?variant":"restricted","#clean":"restricted","#clean?variant":"allowed"}}"##,
            ),
            (
                "a.mjs",
                "import '#clean?variant';\nimport '#banned?variant';",
            ),
            (
                "b.cjs",
                "require('#clean?variant');\nrequire('#banned?variant');",
            ),
        ],
        &[],
        0,
    );
    assert_eq!(
        lines_for(&out, "a.mjs:")
            .into_iter()
            .chain(lines_for(&out, "b.cjs:"))
            .collect::<Vec<_>>(),
        [
            "a.mjs:2:8 boundary/banned-imports [advisory] restricted",
            "b.cjs:2:9 boundary/banned-imports [advisory] restricted",
        ]
    );
}

#[test]
fn package_import_conditions_stop_at_null() {
    let out = judge(
        &[
            (
                "blackhole.toml",
                "[boundaries]\nbanned-imports=['restricted']",
            ),
            (
                "package.json",
                r##"{"imports": {
                    "#blocked": {"node":null,"default":"restricted"},
                    "#blocked-array": {"node":[null],"default":"restricted"},
                    "#array-fallback": {"node":[null,"restricted"],"default":"allowed"}
                }}"##,
            ),
            (
                "a.mjs",
                "import '#blocked';\nimport '#blocked-array';\nimport '#array-fallback';",
            ),
            ("b.cjs", "require('#blocked');"),
            ("c.mts", "import type {Model} from '#blocked';"),
        ],
        &[],
        0,
    );
    assert_eq!(
        out.lines()
            .filter(|line| line.contains(" boundary/banned-imports "))
            .collect::<Vec<_>>(),
        [
            "a.mjs:3:8 boundary/banned-imports [advisory] restricted",
            "c.mts:1:26 boundary/banned-imports [advisory] restricted",
        ]
    );
}

#[test]
fn inline_type_specifiers_load_under_verbatim_module_syntax() {
    let source = "import { type X } from '#alias';\nexport { type X } from '#alias';\nimport type { Y } from '#alias';\nexport type { Y } from '#alias';";
    let out = judge(
        &[
            (
                "blackhole.toml",
                "[boundaries]\nbanned-imports=['restricted']",
            ),
            (
                "package.json",
                r##"{"imports":{"#alias":{"types":"./ok.d.ts","default":"restricted"}}}"##,
            ),
            ("ok.d.ts", "export type X = string; export type Y = string;"),
            (
                "base.json",
                r#"{"compilerOptions":{"verbatimModuleSyntax":true}}"#,
            ),
            ("verbatim/tsconfig.json", r#"{"extends":"../base.json"}"#),
            ("verbatim/a.mts", source),
            ("ordinary/a.mts", source),
        ],
        &[],
        0,
    );
    assert_eq!(
        out.lines()
            .filter(|line| line.contains(" boundary/banned-imports "))
            .collect::<Vec<_>>(),
        [
            "verbatim/a.mts:1:24 boundary/banned-imports [advisory] restricted",
            "verbatim/a.mts:2:24 boundary/banned-imports [advisory] restricted",
        ]
    );
}

#[test]
fn allowed_inline_type_import_skips_unavailable_verbatim_config() {
    let out = judge(
        &[
            (
                "blackhole.toml",
                "[boundaries]\nbanned-imports=['restricted']",
            ),
            (
                "tsconfig.json",
                r#"{"extends":"missing-config/tsconfig.json"}"#,
            ),
            ("a.ts", r#"import { type X } from "./local";"#),
            ("local.ts", "export type X = string;"),
        ],
        &[],
        0,
    );
    assert!(out.is_empty(), "{out}");
}

#[test]
fn loader_sources_share_the_base_check() {
    for (file, declaration, callee, own_base) in [
        ("a.cjs", "", "require.main.require", false),
        ("a.cjs", "", "process.mainModule.require", false),
        ("a.cjs", "", "module.parent.require", false),
        ("a.cjs", "", "module.constructor._load", true),
        ("a.cjs", "const {require:q}=module;", "q", true),
        ("a.cjs", "const {require:q}=globalThis;", "q", true),
        (
            "a.cjs",
            "const {createRequire:f}=require('node:module');",
            "f(__filename)",
            true,
        ),
        (
            "a.mjs",
            "const {createRequire:f}=await import('node:module');",
            "f(import.meta.url)",
            true,
        ),
        (
            "a.mjs",
            "const m=process.getBuiltinModule('node:module');",
            "m.createRequire(import.meta.url)",
            true,
        ),
        (
            "a.mjs",
            "import m from 'node:module';",
            "m.createRequire(import.meta.url)",
            true,
        ),
        ("a.cjs", "const q=require.bind(null);", "q", true),
    ] {
        let event = if Path::new(file).extension().is_some_and(|ext| ext == "mjs") {
            "delete import.meta.url;"
        } else {
            "module.filename='elsewhere';"
        };
        for (mutation, name, note) in [
            ("", "restricted", Some("restricted")),
            ("", "allowed", None),
            (event, "allowed", Some("specifier is not a literal")),
        ] {
            let source = format!("{declaration}\n{mutation}\n{callee}('{name}');");
            let mut expected = Vec::new();
            let unknown = "specifier is not a literal";
            if !mutation.is_empty() {
                let col = if mutation.starts_with("delete") { 8 } else { 1 };
                expected.push((2, col, unknown));
            }
            if !own_base || callee == "module.constructor._load" {
                expected.push((3, 1, unknown));
            } else if declaration.contains("require.bind") {
                expected.push((1, 9, unknown));
            } else if let Some(note) = note.filter(|_| mutation.is_empty()) {
                expected.push((3, callee.len() + 2, note));
            }
            expected.sort_unstable();
            findings(file, &source, &expected);
        }
    }
}

#[test]
fn unreadable_loader_calls_and_module_escapes_are_findings() {
    for source in [
        "const g=globalThis; g['req'+'uire']('allowed');",
        "import * as m from 'node:module'; consume(m);",
        "import * as m from 'module'; const copy={...m};",
        "export * from 'node:module';",
        "import * as m from 'module'; export {m};",
        "const q=require.bind(null,'restricted'); q();",
        "const q=require.bind(null); q('restricted');",
        "const {require:q=other}=module; q('allowed');",
        "const {['req'+'uire']:q}=module; q('allowed');",
        "const {...rest}=module; rest.require('allowed');",
        "require.apply(null, unknown);",
        "require.resolve.apply(null, unknown);",
        "import * as m from 'module'; delete m.createRequire;",
        "const {require:q}=globalThis; consume(q);",
    ] {
        let out = judge_banned("a.ts", source, 0);
        let note = "specifier is not a literal";
        assert!(
            out.contains(&format!("boundary/banned-imports [advisory] {note}\n")),
            "{source}\n{out}"
        );
    }
}

#[test]
fn filename_factory_accepts_implicit_commonjs_javascript() {
    let call =
        "const {createRequire} = require('node:module');\ncreateRequire(__filename)('./x.cjs');";
    for (path, source, code) in [
        ("a.js", call.to_owned(), 0),
        ("a.ts", call.to_owned(), 1),
        ("a.js", format!("{call}\nexport {{}};"), 1),
    ] {
        let expected = [(2, 27, UNKNOWN)];
        findings(path, &source, &expected[..code]);
    }
}

#[test]
fn loader_scope_controls_and_module_pattern_siblings() {
    for source in [
        "eval(\"require('restricted')\");",
        "new Function(\"require('restricted')\");",
        "process.binding('restricted');",
        "const {require:q}=module; require('allowed');",
    ] {
        assert_eq!(judge_banned("a.cjs", source, 0), "");
    }
    for source in [
        "const q=require.bind(null, 'allowed'); q();",
        "module.children;",
        "const {paths}=module; paths.push('../db/node_modules'); require('allowed');",
        "module.paths.push('../db/node_modules'); require('allowed');",
        "import(name);",
        "globalThis[key]('allowed');",
        "const {require:q}=module; consume(q);",
        "let {require:q}=module; q('allowed');",
        "const {...rest}=globalThis; rest.require('restricted');",
        "let q; ({require:q}=module); q('restricted');",
        "var {require:q}=module; q('allowed');",
        "const {require:{call:q}}=module; q('allowed');",
        "let q; ({require:q}=module); require('allowed');",
    ] {
        let out = judge_banned("a.cjs", source, 0);
        assert!(
            out.contains("boundary/banned-imports [advisory] specifier is not a literal"),
            "{source}\n{out}"
        );
    }
}

#[test]
fn instance_and_class_factory_members() {
    // On the CommonJS instance, createRequire is an unknown member.
    for (path, source, positions) in [
        (
            "a.cjs",
            "module.createRequire(__filename)('./x.cjs');",
            &[(1, 1, "specifier is not a literal")][..],
        ),
        (
            "a.cjs",
            "module.createRequire = require('module').createRequire; module.createRequire(__filename)('restricted');",
            &[
                (1, 1, "specifier is not a literal"),
                (1, 24, "specifier is not a literal"),
                (1, 57, "specifier is not a literal"),
            ],
        ),
        (
            "a.mjs",
            "import module from 'node:module'; module.createRequire(import.meta.url)('restricted');",
            &[(1, 73, "restricted")],
        ),
        (
            "a.cjs",
            "module.custom = 1;\nrequire('allowed');",
            &[(1, 1, "specifier is not a literal")],
        ),
        (
            "a.cjs",
            "module.exports.custom = 1;\nrequire('allowed');",
            &[],
        ),
    ] {
        findings(path, source, positions);
    }
}

fn judge_banned(path: &str, source: &str, code: i32) -> String {
    judge(
        &[
            (
                "blackhole.toml",
                "[boundaries]\nbanned-imports=['restricted']",
            ),
            (path, source),
        ],
        &[],
        code,
    )
}

#[test]
fn namespace_binding_positions() {
    let prelude = "import * as m from 'module'; let alias=m;";
    for (tail, position) in [
        ("", None),
        (" consume(alias);", Some(51)),
        (" alias=other;", Some(40)),
    ] {
        let source = format!("{prelude}{tail}");
        let expected = position.map(|col| (1, col, UNKNOWN));
        findings("a.ts", &source, expected.as_slice());
    }
}

#[test]
fn mutable_destructuring_reports_the_factory_initializer() {
    for declaration in ["let", "var"] {
        let source = format!(
            "{declaration} {{ createRequire: f }} = require('module'); f(__filename)('restricted');"
        );
        findings("a.cjs", &source, &[(1, 28, UNKNOWN)]);
    }
}

#[test]
fn unread_loader_positions() {
    for (file, source, needle) in [
        (
            "a.mjs",
            "import {createRequire} from 'module'; consume(createRequire);",
            "createRequire);",
        ),
        ("a.cjs", "consume(process.mainModule);", "process"),
        (
            "a.cjs",
            "let m; m=process.mainModule; m.require('restricted');",
            "process",
        ),
        (
            "a.mjs",
            "import('node:module').then(m=>m.createRequire(import.meta.url)('restricted'))",
            "import(",
        ),
        (
            "a.cjs",
            "const {mainModule:{require:q}}=process; q('restricted')",
            "process",
        ),
        (
            "a.cjs",
            "const {require:q=other}=require.main; q('restricted')",
            "require.main",
        ),
        ("a.cjs", "require.apply(null, unknown);", "require"),
        ("a.cjs", "require.resolve.apply(null, unknown);", "require"),
    ] {
        let col = source.find(needle).unwrap() + 1;
        findings(file, source, &[(1, col, UNKNOWN)]);
    }
}

#[test]
fn process_builtin_and_wrapper_arguments() {
    let config = "[boundaries]\nbanned-imports=['node:path']";
    for (file, source, at, note) in [
        (
            "a.mjs",
            "import {getBuiltinModule as get} from 'node:process'; get('node:path');",
            "'node:path'",
            "node:path",
        ),
        (
            "a.mjs",
            "import p from 'process'; p.getBuiltinModule('node:path');",
            "'node:path'",
            "node:path",
        ),
        (
            "a.mjs",
            "import * as p from 'node:process'; p.getBuiltinModule('node:path');",
            "'node:path'",
            "node:path",
        ),
        (
            "a.cts",
            "import p = require('process'); p.getBuiltinModule('node:path');",
            "'node:path'",
            "node:path",
        ),
        (
            "a.mjs",
            "const p = await import('process'); p.getBuiltinModule('node:path');",
            "'node:path'",
            "node:path",
        ),
        (
            "a.cjs",
            "require('node:process').getBuiltinModule('node:path');",
            "'node:path'",
            "node:path",
        ),
        (
            "a.cjs",
            "const {mainModule} = require('process'); mainModule.require('allowed');",
            "mainModule.require",
            "specifier is not a literal",
        ),
        (
            "a.cjs",
            "process.getBuiltinModule('node:process').getBuiltinModule('node:path');",
            "'node:path'",
            "node:path",
        ),
        (
            "a.cjs",
            "arguments[1]('restricted');",
            "arguments",
            "specifier is not a literal",
        ),
        (
            "a.cjs",
            "const [,r] = arguments; r('restricted');",
            "arguments",
            "specifier is not a literal",
        ),
    ] {
        let out = judge(&[("blackhole.toml", config), (file, source)], &[], 0);
        let col = source.find(at).unwrap() + 1;
        assert_eq!(
            lines_for(&out, file),
            [format!(
                "{file}:1:{col} boundary/banned-imports [advisory] {note}"
            )],
            "{source}"
        );
    }
}

#[test]
fn trusted_base_uses() {
    for (file, source, at) in [
        ("a.cjs", "__filename = 'elsewhere';", "__filename"),
        ("a.cjs", "__filename += '/elsewhere';", "__filename"),
        ("a.cjs", "__filename++;", "__filename"),
        ("a.cjs", "delete __filename;", "__filename"),
        ("a.cjs", "({x: __filename} = src);", "__filename"),
        ("a.cjs", "for (__filename of names) {}", "__filename"),
        ("a.mjs", "import.meta.url = 'elsewhere';", "import.meta"),
        ("a.mjs", "import.meta.url += '/elsewhere';", "import.meta"),
        ("a.mjs", "import.meta.url++;", "import.meta"),
        ("a.mjs", "delete import.meta.url;", "import.meta"),
        ("a.mjs", "({x: import.meta.url} = src);", "import.meta"),
        ("a.mjs", "for (import.meta.url of names) {}", "import.meta"),
        (
            "a.mjs",
            "Object.assign(import.meta, { url });",
            "import.meta",
        ),
        (
            "a.cjs",
            "var __filename = '/other/a.cjs'; require('module').createRequire(__filename)('./x.cjs');",
            "'./x.cjs'",
        ),
    ] {
        let col = source.find(at).unwrap() + 1;
        findings(file, source, &[(1, col, UNKNOWN)]);
    }
    for (file, source) in [
        (
            "a.cjs",
            "console.log(__filename); path.dirname(__filename); function f(__filename) { __filename++; }",
        ),
        (
            "a.mjs",
            "import {createRequire} from 'module'; createRequire(import.meta.url)('allowed'); const {url} = import.meta; import.meta.env.X; import.meta.resolve('allowed');",
        ),
    ] {
        assert_eq!(judge_banned(file, source, 0), "");
    }
}

#[test]
fn base_write_reports_only_at_the_write() {
    for (file, source) in [
        (
            "ui/a.cjs",
            "const f = require('node:module').createRequire;\n__filename = require('node:path').resolve(__dirname, '../db/a.cjs');\nf(__filename)('./x.cjs');",
        ),
        (
            "ui/a.mjs",
            "import {createRequire as f} from 'node:module';\nimport.meta.url = new URL('../db/a.cjs', import.meta.url).href;\nf(import.meta.url)('./x.cjs');",
        ),
        (
            "ui/b.mjs",
            "import {createRequire as f} from 'node:module';\nimport.meta.filename = new URL('../db/b.mjs', import.meta.url).pathname;\nf(import.meta.filename)('./x.cjs');",
        ),
    ] {
        let out = judge(
            &[
                (
                    "blackhole.toml",
                    "[boundaries]\nbanned-imports=['restricted']",
                ),
                (file, source),
                ("ui/x.cjs", ""),
                ("db/x.cjs", ""),
            ],
            &[],
            0,
        );
        assert_eq!(
            lines_for(&out, file),
            [format!(
                "{file}:2:1 boundary/banned-imports [advisory] specifier is not a literal"
            )]
        );
    }
}

#[test]
fn filename_base_in_explicit_modules() {
    let source = "import {createRequire} from 'node:module'; createRequire(__filename)('allowed');";
    for file in ["a.mjs", "a.mts"] {
        let col = source.find("'allowed'").unwrap() + 1;
        findings(file, source, &[(1, col, UNKNOWN)]);
    }
}

#[test]
fn filename_global_property_writes() {
    for target in ["globalThis.__filename", "global['__filename']"] {
        for statement in [
            format!("{target} = path;"),
            format!("{target} += path;"),
            format!("{target}++;"),
            format!("delete {target};"),
            format!("({{x: {target}}} = src);"),
            format!("for ({target} of names) {{}}"),
        ] {
            let source = format!("{statement}\nrequire('allowed');");
            let col = statement.find(target).unwrap() + 1;
            findings("a.cjs", &source, &[(1, col, UNKNOWN)]);
        }
    }
}

#[test]
fn global_filename_is_not_a_wrapper_base_in_module_packages() {
    let source = "import { createRequire } from 'node:module';\nObject.assign(globalThis, { __filename: new URL('../db/a.js', import.meta.url).pathname });\nconsole.log(createRequire(__filename)('./x.cjs'));";
    let out = judge(
        &[
            (
                "blackhole.toml",
                "[boundaries]\nbanned-imports=['restricted']",
            ),
            ("package.json", r#"{"type":"module"}"#),
            ("ui/a.js", source),
            ("ui/x.cjs", ""),
            ("db/x.cjs", ""),
        ],
        &[],
        0,
    );
    assert_eq!(
        lines_for(&out, "ui/a.js:"),
        ["ui/a.js:3:39 boundary/banned-imports [advisory] specifier is not a literal"]
    );
}

#[test]
fn closed_loader_forms() {
    use std::fmt::Write;
    let mut source = String::from(
        "import m, {createRequire as factory} from 'node:module';\nconst load=factory(__filename);\n",
    );
    let mut expected = Vec::new();
    let mut line = 2;
    for root in [
        "require",
        "module",
        "module.require",
        "load",
        "m",
        "factory",
        "process.getBuiltinModule",
        "process.mainModule",
        "require.main",
        "arguments",
    ] {
        for template in [
            "L.call(null,'allowed');",
            "L.apply(null,['allowed']);",
            "L.bind(null)('allowed');",
            "Reflect.apply(L,null,['allowed']);",
            "consume(L);",
            "[L];",
            "({value:L});",
            "(()=>L)();",
            "[...L];",
        ] {
            line += 1;
            let col = template.find('L').unwrap() + 1;
            source.push_str(&template.replace('L', root));
            source.push('\n');
            if template.starts_with("Reflect.apply") {
                expected.push(format!("a.cts:{line}:1 ts/no-reflect-apply"));
            }
            expected.push(format!(
                "a.cts:{line}:{col} boundary/banned-imports [advisory] specifier is not a literal"
            ));
        }
    }
    for root in [
        "require",
        "module.require",
        "module['require']",
        "load",
        "factory(__filename)",
        "process.getBuiltinModule",
    ] {
        for name in ["allowed", "restricted"] {
            line += 1;
            writeln!(source, "{root}('{name}');").unwrap();
            if name == "restricted" {
                expected.push(format!(
                    "a.cts:{line}:{} boundary/banned-imports [advisory] restricted",
                    root.len() + 2
                ));
            }
        }
    }
    let actual = judge_banned("a.cts", &source, 1);
    assert_eq!(lines_for(&actual, "a.cts:"), expected);
    check_loader_spellings();
    check_exported_loaders();
}

fn check_loader_spellings() {
    for (stmt, at, note) in [
        (
            "const r = module?.require; r('restricted');",
            "'restricted'",
            "restricted",
        ),
        (
            "(module?.require)('restricted');",
            "'restricted'",
            "restricted",
        ),
        (
            "(module?.require)?.('restricted');",
            "'restricted'",
            "restricted",
        ),
        (
            "const r = (module)?.require; r('restricted');",
            "'restricted'",
            "restricted",
        ),
        (
            "const {require:r} = globalThis?.module; r('restricted');",
            "'restricted'",
            "restricted",
        ),
        (
            "const p = globalThis?.process; p.getBuiltinModule('restricted');",
            "'restricted'",
            "restricted",
        ),
        (
            "globalThis.globalThis.require('restricted');",
            "'restricted'",
            "restricted",
        ),
        (
            "globalThis.global.require('restricted');",
            "'restricted'",
            "restricted",
        ),
        (
            "global.global.process.mainModule.require('restricted');",
            "global.global.process",
            "specifier is not a literal",
        ),
        (
            "global['globalThis'].global.__filename = 'x';",
            "global[",
            "specifier is not a literal",
        ),
        (
            "require('module').createRequire(somewhere).main.require('restricted');",
            "require('module')",
            "specifier is not a literal",
        ),
        (
            "require(require('module').createRequire(somewhere).resolve(dyn));",
            "require('module')",
            "specifier is not a literal",
        ),
        (
            "const r = require('module').createRequire(somewhere); r(r.resolve(dyn));",
            "r.resolve",
            "specifier is not a literal",
        ),
        (
            "require(require.resolve('restricted'));",
            "require.resolve",
            "specifier is not a literal",
        ),
        (
            "const r = require('node:module').createRequire(__filename); r(r['resolve']('restricted'));",
            "r['resolve']",
            "specifier is not a literal",
        ),
        (
            "with (process) { mainModule.require('restricted'); }",
            "process",
            "specifier is not a literal",
        ),
        (
            "with (process) { getBuiltinModule('restricted'); }",
            "process",
            "specifier is not a literal",
        ),
        (
            "with (globalThis) { require('restricted'); }",
            "globalThis",
            "specifier is not a literal",
        ),
    ] {
        let output = judge_banned("row.cjs", stmt, 0);
        let col = stmt.find(at).unwrap() + 1;
        let expected = format!("row.cjs:1:{col} boundary/banned-imports [advisory] {note}");
        assert!(output.contains(&expected), "{stmt}\n{output}");
    }
}

fn check_exported_loaders() {
    let prelude = "import {createRequire} from 'node:module'; ";
    for (path, source, columns) in [
        ("a.mts", "export default require;".to_owned(), vec![16]),
        ("a.mts", "export default module;".into(), vec![16]),
        ("a.mts", "export default module.require;".into(), vec![16]),
        (
            "a.mts",
            "export default process.mainModule;".into(),
            vec![16],
        ),
        (
            "a.mts",
            "export default process.getBuiltinModule;".into(),
            vec![16],
        ),
        (
            "a.mts",
            format!("{prelude}export default createRequire;"),
            vec![59],
        ),
        (
            "a.mts",
            format!("{prelude}export default createRequire(import.meta.url);"),
            vec![59],
        ),
        ("a.mts", "const r = require; export { r };".into(), vec![29]),
        (
            "a.mts",
            "const r = require; export { r as load };".into(),
            vec![29],
        ),
        (
            "a.mts",
            "const r = require; export default r;".into(),
            vec![35],
        ),
        ("a.mts", "export const r = require;".into(), vec![18]),
        (
            "a.mts",
            format!(
                "{prelude}export const load = createRequire(import.meta.url); load('restricted');"
            ),
            vec![64, 101],
        ),
        ("a.cts", "export = require;".into(), vec![10]),
        ("a.cts", "export = module;".into(), vec![10]),
        (
            "a.mts",
            "export { createRequire } from 'node:module';".into(),
            vec![31],
        ),
        ("a.mts", "export * from 'node:module';".into(), vec![15]),
    ] {
        let expected: Vec<_> = columns
            .into_iter()
            .map(|col| (1, col, if col == 101 { "restricted" } else { UNKNOWN }))
            .collect();
        findings(path, &source, &expected);
    }
}

#[test]
fn ordinary_module_reads() {
    let source = "import {builtinModules, isBuiltin, createRequire} from 'node:module';
module.exports.value = module.exports.other; module.id; module.hot.accept('allowed');
require.resolve('allowed'); require.main === module;
typeof require === 'function' && require('allowed'); function f(){ require('allowed'); return arguments[0]; }
createRequire(import.meta.url)('allowed'); builtinModules.includes('fs'); isBuiltin('fs');
const {env}=process; (globalThis as any).x=1; require.length; createRequire['name'];";
    assert_eq!(
        lines_for(&judge_banned("a.ts", source, 1), "a.ts:"),
        [
            "a.ts:4:1 ts/no-runtime-typeof",
            "a.ts:6:37 ts/no-explicit-any"
        ]
    );
    let source = "if (process.mainModule === module) { main(); }
typeof process.mainModule; typeof require.main;
type R = typeof require; let r: typeof require; let m: typeof module; type P = typeof process.mainModule;
type M = typeof import('node:module');";
    assert_eq!(
        lines_for(&judge_banned("a.cts", source, 1), "a.cts:"),
        [
            "a.cts:2:1 ts/no-runtime-typeof",
            "a.cts:2:28 ts/no-runtime-typeof"
        ]
    );
    for source in [
        "export default module.exports;",
        "export const id = module.id;",
        "export const here = import.meta.url;",
        "export type R = typeof require;",
        "export default process.env;",
        "export { env } from 'node:process';",
        "export default process;",
        "export default globalThis;",
    ] {
        assert_eq!(judge_banned("a.mts", source, 0), "");
    }
}

#[test]
fn require_resolve_reports_only_when_the_result_reaches_a_loader() {
    let out = judge_banned(
        "a.cjs",
        "require.resolve(name);\nrequire(require.resolve(name));",
        0,
    );
    assert_eq!(
        lines_for(&out, "a.cjs:"),
        ["a.cjs:2:9 boundary/banned-imports [advisory] specifier is not a literal"]
    );
}

#[test]
fn dynamic_import_of_lookup_reports_non_literal() {
    let out = judge(
        &[
            (
                "blackhole.toml",
                "[boundaries]\nbanned-imports=['restricted']",
            ),
            (
                "ui/load.cjs",
                "import(require.resolve('./x'));\nimport(require.resolve('restricted'));",
            ),
            ("ui/load.mjs", "await import(require.resolve('./a.mjs'));"),
            (
                "ui/computed.cjs",
                "import(require['resolve']('restricted'));",
            ),
            (
                "ui/alias.cjs",
                "const lookup = require.resolve; import(lookup('restricted'));",
            ),
            ("ui/a.mjs", "import './load.mjs';"),
            ("ui/x.js", "import './load.cjs';"),
            (
                "node_modules/restricted/package.json",
                r#"{"exports":{"import":"./db/x.mjs","require":"./ok.cjs"}}"#,
            ),
            ("node_modules/restricted/db/x.mjs", "export default 1;"),
            ("node_modules/restricted/ok.cjs", "module.exports=1;"),
        ],
        &[],
        0,
    );
    assert_eq!(
        lines_for(&out, "ui/"),
        [
            "ui/alias.cjs:1:40 boundary/banned-imports [advisory] specifier is not a literal",
            "ui/computed.cjs:1:8 boundary/banned-imports [advisory] specifier is not a literal",
            "ui/load.cjs:1:8 boundary/banned-imports [advisory] specifier is not a literal",
            "ui/load.cjs:2:8 boundary/banned-imports [advisory] specifier is not a literal",
            "ui/load.mjs:1:14 boundary/banned-imports [advisory] specifier is not a literal",
        ]
    );
}

#[test]
fn lookup_with_options_is_reported_when_loaded() {
    let source = "require(require.resolve('restricted', { paths: [dir] }));";
    findings("a.cjs", source, &[(1, 9, UNKNOWN)]);
}

#[test]
fn import_meta_lookup_is_reported_when_loaded() {
    for source in [
        "await import(import.meta.resolve('restricted'));",
        "await import(import.meta.resolve(dyn));",
    ] {
        findings("a.mjs", source, &[(1, 14, UNKNOWN)]);
    }
}

#[test]
fn loaded_lookup_reports_for_either_loader_base() {
    for expr in [
        "createRequire(__filename)(createRequire(import.meta.url).resolve('restricted'));",
        "createRequire(__filename)(createRequire(import.meta.filename).resolve('restricted'));",
        "createRequire(__filename)(createRequire(import.meta['url'])['resolve']('restricted'));",
        "createRequire(import.meta.url)(createRequire(__filename).resolve('restricted'));",
    ] {
        let source = format!("import {{createRequire}} from 'node:module';\n{expr}");
        let col = expr.find("(createRequire").unwrap() + 2;
        findings("a.mjs", &source, &[(2, col, UNKNOWN)]);
    }
}

#[test]
fn builtin_loader_reports_resolved_argument() {
    let source = "process.getBuiltinModule(require.resolve('node:module')).createRequire(__filename)('restricted');";
    findings("a.cjs", source, &[(1, 26, UNKNOWN)]);
}

#[test]
fn destructured_lookup_is_reported_when_loaded() {
    for (source, note) in [
        (
            "const {createRequire} = require('node:module');\nconst { resolve } = createRequire(dyn);\nrequire(resolve(dyn2));",
            "specifier is not a literal",
        ),
        (
            "const { resolve } = require;\nrequire(resolve('restricted'));",
            "specifier is not a literal",
        ),
    ] {
        let out = judge_banned("a.cjs", source, 0);
        assert!(
            lines_for(&out, "a.cjs:").contains(
                &format!(
                    "a.cjs:{}:9 boundary/banned-imports [advisory] {note}",
                    source.lines().count()
                )
                .as_str()
            ),
            "{source}\n{out}"
        );
    }
}

#[test]
fn unknown_base_computed_lookup_reports_only_when_loaded() {
    let setup = "const r = require('node:module').createRequire(dyn);";
    let alone = format!("{setup}\nr['resolve'](dyn);");
    assert_eq!(judge_banned("a.cjs", &alone, 0), "");
    let loaded = format!("{setup}\nrequire(r['resolve'](dyn));");
    findings("a.cjs", &loaded, &[(2, 9, UNKNOWN)]);
}

#[test]
fn unknown_base_lookup_reads_report_at_the_member() {
    let setup = "const r = require('node:module').createRequire(dyn);";
    for (read, col) in [
        ("const f = r.resolve;", 11),
        ("const f = (r.resolve).foo;", 12),
    ] {
        let source = format!("{setup}\n{read}");
        findings("a.cjs", &source, &[(2, col, UNKNOWN)]);
    }
}

#[test]
fn resolved_load_reports_at_its_argument_start() {
    let source = "require('restricted');\nrequire(require.resolve('restricted'));";
    findings("a.cjs", source, &[(1, 9, "restricted"), (2, 9, UNKNOWN)]);
}

#[test]
fn runtime_builtins_loaded_through_resolve_report_at_the_load() {
    for (path, argument, source) in [
        (
            "a.cjs",
            "require.resolve",
            "require(require.resolve('module')).createRequire(__filename)('restricted');",
        ),
        (
            "a.cjs",
            "lookup('node:module')",
            "const lookup = require.resolve; const m = require(lookup('node:module')); m.createRequire(__filename)('restricted');",
        ),
        (
            "a.mjs",
            "r.resolve",
            "import {createRequire} from 'node:module'; const r = createRequire(import.meta.url); const m = await import(r.resolve('node:module')); m.createRequire(import.meta.url)('restricted');",
        ),
    ] {
        let col = source.find(argument).unwrap() + 1;
        findings(path, source, &[(1, col, UNKNOWN)]);
    }
}

#[test]
fn allowed_lookup_names_are_still_non_literal_load_arguments() {
    for (path, source, argument) in [
        ("a.cjs", "import(require.resolve('p'));", "require.resolve"),
        (
            "a.ts",
            "const r = require('node:module').createRequire(__filename); r(require.resolve('node:path'));",
            "require.resolve",
        ),
        (
            "a.cjs",
            "const path = module.require['resolve']('p'); require(path);",
            "path);",
        ),
    ] {
        let col = source.find(argument).unwrap() + 1;
        findings(path, source, &[(1, col, UNKNOWN)]);
    }
}

#[test]
fn destructuring_a_lookup_does_not_load_anything() {
    for source in [
        "const {resolve: lookup} = module.require; lookup(name);",
        "const {resolve} = require; const lookup = resolve; lookup('restricted');",
    ] {
        assert_eq!(judge_banned("a.ts", source, 0), "", "{source}");
    }
}

#[test]
fn destructuring_a_lookup_with_an_untrusted_base_reports_at_the_binding() {
    let source = "import {createRequire} from 'node:module';\nconst {resolve} = createRequire(__filename + ''); resolve('node:path');";
    findings("a.ts", source, &[(2, 19, UNKNOWN)]);
}

#[path = "boundary/runtime.rs"]
mod runtime;

#[test]
fn scoped_check_resolves_aliases_outside_the_selection() {
    let dir = cli::repo();
    let root = dir.path();
    put(
        root,
        "blackhole.toml",
        "[boundaries]\nbanned-imports=['restricted']",
    );
    put(
        root,
        "package.json",
        r##"{"imports":{"#entry":"restricted"}}"##,
    );
    put(root, "src/app.mts", "import '#entry';");
    put(root, "other.mts", "import 'restricted';");
    let output = cli::check(root, &["src"], 0);
    let text = String::from_utf8(output.stdout).unwrap();
    assert_eq!(
        text.lines()
            .filter(|line| line.contains(" boundary/banned-imports "))
            .collect::<Vec<_>>(),
        ["src/app.mts:1:8 boundary/banned-imports [advisory] restricted"]
    );
}

#[test]
fn package_metadata_keys_are_plain_keys() {
    let package = r##"{"meta":[{"$serde_json::private::Number":"x"},{"$serde_json::private::RawValue":"x"}],"imports":{"#entry":"restricted"}}"##;
    let out = judge(
        &[
            (
                "blackhole.toml",
                "[boundaries]\nbanned-imports=['restricted']",
            ),
            ("package.json", package),
            ("app.mts", "import '#entry';"),
        ],
        &[],
        0,
    );
    assert_eq!(
        lines_for(&out, "app.mts"),
        ["app.mts:1:8 boundary/banned-imports [advisory] restricted"]
    );
}

#[test]
fn unreadable_resolver_configuration_is_advisory_across_commands() {
    let dir = cli::repo();
    let root = dir.path();
    put(root, "package.json/placeholder", "");
    put(root, "app.mjs", "import '#entry';");
    for (bans, expected) in [
        ("[]", None),
        (
            "['restricted']",
            Some(
                "app.mjs:1:8 boundary/banned-imports [advisory] \"package.json\": listed configuration is a directory",
            ),
        ),
    ] {
        put(
            root,
            "blackhole.toml",
            &format!("[boundaries]\nbanned-imports={bans}"),
        );
        common::git(root, &["add", "-f", "."]).unwrap();
        common::git(root, &["commit", "-qm", "case"]).unwrap();
        for args in [
            &["check"][..],
            &["ci", "--base", "HEAD", "--head", "HEAD"],
            &["ledger"],
        ] {
            let output = cli::run(root, args, 0);
            let stdout = String::from_utf8(output.stdout).unwrap();
            assert_eq!(
                lines_for(&stdout, "app.mjs:"),
                expected.as_slice(),
                "{args:?}"
            );
        }
    }
}
