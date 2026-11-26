use super::{judge, lines_for};

const FOLDERS: &str = "[[boundaries.folders]]\nfrom='ui/**'\ndeny=['db/**']";

#[test]
fn literal_require_follows_encoded_filename() {
    let out = judge(
        &[
            ("blackhole.toml", FOLDERS),
            ("ui/a.cjs", "require('./%78.cjs');"),
            ("ui/x.cjs", ""),
            ("db/x.cjs", ""),
        ],
        &[("ui/%78.cjs", "../db/x.cjs")],
        0,
    );
    assert!(out.contains("ui/a.cjs:1:9 boundary/folder-imports [advisory] db/x.cjs"));
}

#[test]
fn decoded_delimiters_name_files() {
    for (encoded, ch) in [("%3F", "?"), ("%23", "#")] {
        let out = judge(
            &[
                ("blackhole.toml", FOLDERS),
                ("ui/a.mjs", &format!("import './{encoded}x.mjs';")),
                ("db/x.mjs", ""),
            ],
            &[(&format!("ui/{ch}x.mjs"), "../db/x.mjs")],
            0,
        );
        assert!(out.contains("ui/a.mjs:1:8 boundary/folder-imports [advisory] db/x.mjs"));
    }
}

#[test]
fn url_backslashes_separate_directories() {
    let out = judge(
        &[
            ("blackhole.toml", FOLDERS),
            ("ui/a.mjs", r"import './..\x5cdb\x5cx.mjs';"),
            ("db/x.mjs", ""),
        ],
        &[],
        0,
    );
    assert!(out.contains("ui/a.mjs:1:8 boundary/folder-imports [advisory] db/x.mjs"));
}

#[test]
fn link_directory_suffix_requires_a_directory() {
    for suffix in ["/", "/."] {
        let dir = super::repository(
            &[
                ("ui/a.mjs", "import './alias.mjs';"),
                ("db/x.mjs", ""),
                ("blackhole.toml", FOLDERS),
            ],
            &[("ui/alias.mjs", &format!("../db/x.mjs{suffix}"))],
        );
        let out = super::cli::check(dir.path(), &[], 0);
        let stdout = String::from_utf8(out.stdout).unwrap();
        assert_eq!(
            lines_for(&stdout, "ui/a.mjs:"),
            [
                "ui/a.mjs:1:8 boundary/folder-imports [advisory] unreadable repository link: ui/alias.mjs"
            ]
        );
        let out = super::cli::run(dir.path(), &["ci", "--base", "HEAD", "--head", "HEAD"], 2);
        assert_eq!(
            String::from_utf8(out.stderr).unwrap(),
            "blackhole: \"ui/alias.mjs\": link target is not in the commit\n"
        );
        let out = judge(
            &[
                ("ui/a.mjs", "import './alias/x.mjs';"),
                ("db/x.mjs", ""),
                ("blackhole.toml", FOLDERS),
            ],
            &[("ui/alias", &format!("../db{suffix}"))],
            0,
        );
        assert!(out.contains("ui/a.mjs:1:8 boundary/folder-imports [advisory] db/x.mjs"));
    }
}

#[test]
fn missing_import_uses_real_importer_directory() {
    let out = judge(
        &[
            (
                "blackhole.toml",
                "[[boundaries.folders]]\nfrom='ui/**'\ndeny=['src/deep/missing.mjs']",
            ),
            ("src/deep/a.mjs", "import './missing.mjs';"),
            ("ui/keep", ""),
        ],
        &[("ui/a.mjs", "../src/deep/a.mjs")],
        0,
    );
    assert!(out.contains("ui/a.mjs:1:8 boundary/folder-imports [advisory] src/deep/missing.mjs"));
}

#[test]
fn unknown_factory_base_reports_argument_start() {
    let out = judge(
        &[
            ("blackhole.toml", FOLDERS),
            (
                "ui/a.ts",
                "import {createRequire as f} from 'node:module';\nf(__dirname)(('ok' as string));",
            ),
        ],
        &[],
        0,
    );
    assert!(
        out.contains("ui/a.ts:2:14 boundary/folder-imports [advisory] specifier is not a literal")
    );
}

#[test]
fn factory_requires_unmodified_module_base() {
    for change in [
        "__filename = 'elsewhere';",
        "const __filename = 'elsewhere';",
    ] {
        let source = format!(
            "const f = module.constructor.createRequire;\n{{ {change}\nf(__filename)('./x.cjs'); }}"
        );
        let out = judge(
            &[("blackhole.toml", FOLDERS), ("ui/a.cjs", &source)],
            &[],
            0,
        );
        assert!(
            out.contains(
                "ui/a.cjs:1:11 boundary/folder-imports [advisory] specifier is not a literal"
            ),
            "{out}"
        );
    }
    for mutation in [
        "import.meta.url = new URL('../db/a.cjs', import.meta.url).href;",
        "import.meta['url'] = 'elsewhere';",
        "Object.assign(import.meta, {url: 'elsewhere'});",
        "const m = import.meta; m.url = 'elsewhere';",
        "const {...m} = import.meta;",
        "const m = {...import.meta};",
        "delete import.meta.url;",
    ] {
        let source = format!(
            "import {{createRequire as f}} from 'node:module';\n{mutation}\nf(import.meta.url)('./x.cjs');"
        );
        let out = judge(
            &[("blackhole.toml", FOLDERS), ("ui/a.mjs", &source)],
            &[],
            0,
        );
        assert!(
            out.contains(&format!(
                "ui/a.mjs:2:{} boundary/folder-imports [advisory] specifier is not a literal",
                mutation.find("import.meta").unwrap() + 1
            )),
            "{out}"
        );
    }
}

#[test]
fn loaders_require_a_trusted_base() {
    for (ext, prelude, load) in [
        (
            "mjs",
            "globalThis.__filename = new URL('../db/a.cjs', import.meta.url).pathname;",
            "module.constructor.createRequire(__filename)('./x.cjs')",
        ),
        (
            "cjs",
            "arguments[3] = 'elsewhere';",
            "module.constructor.createRequire(__filename)('./x.cjs')",
        ),
        (
            "cjs",
            "eval(\"__filename = 'elsewhere'\");",
            "module.constructor.createRequire(__filename)('./x.cjs')",
        ),
        (
            "cjs",
            "module.filename = '/db/index.cjs';",
            "require('./x.cjs')",
        ),
        (
            "mjs",
            "delete import.meta?.url;",
            "module.constructor.createRequire(import.meta.url)('./x.cjs')",
        ),
        (
            "js",
            "",
            "module.constructor.createRequire(__filename)('./x.cjs')",
        ),
        (
            "cts",
            "const key = '__filename';",
            "module.constructor.createRequire(__filename)('./x.cjs')",
        ),
        (
            "cjs",
            "const alias = module; alias.filename = '/db/index.cjs';",
            "require('./x.cjs')",
        ),
        ("cjs", "consume(module);", "require('./x.cjs')"),
    ] {
        let out = judge(
            &[
                ("blackhole.toml", FOLDERS),
                (&format!("ui/a.{ext}"), &format!("{prelude}\n{load};")),
            ],
            &[],
            0,
        );
        assert!(
            out.contains("boundary/folder-imports [advisory] specifier is not a literal"),
            "{out}"
        );
    }
    for (safe, others) in [
        ("const alias = module;", &[][..]),
        ("typeof module;", &["ui/a.cjs:1:1 ts/no-runtime-typeof"][..]),
        ("require.main === module;", &[]),
        ("module.exports.x = 1;", &[]),
        ("function f() { return arguments.length; }", &[]),
    ] {
        let out = judge(
            &[
                ("blackhole.toml", FOLDERS),
                (
                    "ui/a.cjs",
                    &format!("{safe}\nrequire('module').createRequire(__filename)('./x.cjs');"),
                ),
            ],
            &[],
            i32::from(!others.is_empty()),
        );
        assert_eq!(lines_for(&out, "ui/a.cjs:"), others, "{out}");
    }
}

#[test]
fn url_readings_cover_controls_schemes_and_all_path_shapes() {
    let written = [
        r"./d\tb/x",
        r"./d\nb/x",
        r"./d\rb/x",
        r"\x01./ok",
        "./ok ",
        " FiLe:x",
        "DaTa:x",
        "Http:x",
        "HTTPS:x",
        "bLoB:x",
        "NODE:fs",
        "pkg/%78",
        "@app/db/%78",
        r"pkg\\x",
        r"/src\\db/%78",
        "virtual:%78",
        "bun:%78",
        "astro:%78",
        "npm:%78",
    ];
    let source = written.map(|name| format!("import '{name}';")).join("\n");
    let policy = "[boundaries]\nbanned-imports=['fs','pkg/x','@app/db/x','/src/db/x']\n[[boundaries.folders]]\nfrom='ui/**'\ndeny=['db/**']";
    let out = judge(&[("blackhole.toml", policy), ("ui/a.mjs", &source)], &[], 0);
    for line in 1..=15 {
        assert!(
            out.contains(&format!("ui/a.mjs:{line}:8 ")),
            "line {line}: {out}"
        );
    }
    for line in 7..=9 {
        for rule in ["banned-imports", "folder-imports"] {
            assert!(
                out.contains(&format!("ui/a.mjs:{line}:8 boundary/{rule} ")),
                "{out}"
            );
        }
    }
    for line in 16..=19 {
        assert!(!out.contains(&format!("ui/a.mjs:{line}:8 ")), "{out}");
    }
}

#[test]
fn extension_alias_preserves_denied_written_path() {
    let out = judge(
        &[
            (
                "blackhole.toml",
                "[[boundaries.folders]]\nfrom='ui/**'\ndeny=['db/x.js']",
            ),
            ("ui/a.ts", "import '../db/x.js';"),
            ("db/x.js", ""),
            ("db/x.ts", ""),
        ],
        &[],
        0,
    );
    assert!(out.contains("ui/a.ts:1:8 boundary/folder-imports"));
}

#[test]
fn static_typescript_import_checks_both_package_conditions() {
    let source = "import value from 'pkg'; console.log(value);";
    let output = judge(
        &[
            (
                "blackhole.toml",
                "[[boundaries.folders]]\nfrom='ui/**'\ndeny=['node_modules/pkg/db/**']",
            ),
            ("ui/a.ts", source),
            ("ui/b.mts", source),
            ("ui/c.cts", source),
            (
                "node_modules/pkg/package.json",
                r#"{"name":"pkg","exports":{"import":"./ok.mjs","require":"./db/x.cjs"}}"#,
            ),
            ("node_modules/pkg/ok.mjs", "export default 1;"),
            ("node_modules/pkg/db/x.cjs", "module.exports=1;"),
        ],
        &[],
        0,
    );
    assert_eq!(
        lines_for(&output, "ui/"),
        [
            "ui/a.ts:1:19 boundary/folder-imports [advisory] node_modules/pkg/db/x.cjs",
            "ui/c.cts:1:19 boundary/folder-imports [advisory] node_modules/pkg/db/x.cjs"
        ]
    );
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
                ("blackhole.toml", FOLDERS),
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
                "{file}:2:1 boundary/folder-imports [advisory] specifier is not a literal"
            )]
        );
    }
}

#[test]
fn global_filename_is_not_a_wrapper_base_in_module_packages() {
    let source = "import { createRequire } from 'node:module';\nObject.assign(globalThis, { __filename: new URL('../db/a.js', import.meta.url).pathname });\nconsole.log(createRequire(__filename)('./x.cjs'));";
    let out = judge(
        &[
            ("blackhole.toml", FOLDERS),
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
        ["ui/a.js:3:39 boundary/folder-imports [advisory] specifier is not a literal"]
    );
}

#[test]
fn type_imports_follow_the_file_or_explicit_resolution_mode() {
    for (file, template, denied) in [
        ("ui/a.cts", "import type {Foo} from 'PKG';", true),
        ("ui/a.MTS", "import type {Foo} from 'PKG';", false),
        ("ui/a.ts", "export type {Foo} from 'PKG';", true),
        ("ui/a.tsx", "export {type Foo} from 'PKG';", true),
        ("ui/a.CTS", "import 'PKG';", true),
        ("ui/a.Cjs", "require('PKG');", true),
        (
            "ui/a.mts",
            "import type {Foo} from 'PKG' with {'resolution-mode':'require'};",
            true,
        ),
        (
            "ui/a.cts",
            "export type {Foo} from 'PKG' with {'resolution-mode':'import'};",
            false,
        ),
        (
            "ui/a.mts",
            "export type * from 'PKG' with {'resolution-mode':'require'};",
            true,
        ),
        ("ui/a.cts", "type T = import('PKG').Foo;", true),
        (
            "ui/a.mts",
            "type T = import('PKG', {with:{'resolution-mode':'require'}}).Foo;",
            true,
        ),
        (
            "ui/a.cts",
            "type T = typeof import('PKG', {with:{'resolution-mode':'import'}});",
            false,
        ),
    ] {
        for package in ["#choice", "pkg"] {
            let source = template.replace("PKG", package);
            let out = judge(
                &[
                    (
                        "blackhole.toml",
                        "[[boundaries.folders]]\nfrom='ui/**'\ndeny=['db/**','node_modules/pkg/db/**']",
                    ),
                    (
                        "package.json",
                        r##"{"imports":{"#choice":{"import":"./ok.mjs","require":"./db/x.cjs"}}}"##,
                    ),
                    (
                        "node_modules/pkg/package.json",
                        r#"{"name":"pkg","exports":{"import":"./ok.mjs","require":"./db/x.cjs"}}"#,
                    ),
                    ("ok.mjs", ""),
                    ("db/x.cjs", ""),
                    ("node_modules/pkg/ok.mjs", ""),
                    ("node_modules/pkg/db/x.cjs", ""),
                    (file, &source),
                ],
                &[],
                0,
            );
            let target = if package == "pkg" {
                "node_modules/pkg/db/x.cjs"
            } else {
                "db/x.cjs"
            };
            let expected = denied.then(|| {
                format!(
                    "{file}:1:{} boundary/folder-imports [advisory] {target}",
                    source.find(package).unwrap()
                )
            });
            assert_eq!(
                lines_for(&out, file),
                expected.iter().collect::<Vec<_>>(),
                "{source}"
            );
        }
    }
}

#[test]
fn dynamic_import_of_lookup_reports_non_literal() {
    let out = judge(
        &[
            (
                "blackhole.toml",
                "[boundaries]\nbanned-imports=['restricted']\n[[boundaries.folders]]\nfrom='ui/**'\ndeny=['node_modules/restricted/db/**', 'ui/a.mjs']",
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
            "ui/alias.cjs:1:40 boundary/folder-imports [advisory] specifier is not a literal",
            "ui/computed.cjs:1:8 boundary/banned-imports [advisory] specifier is not a literal",
            "ui/computed.cjs:1:8 boundary/folder-imports [advisory] specifier is not a literal",
            "ui/load.cjs:1:8 boundary/banned-imports [advisory] specifier is not a literal",
            "ui/load.cjs:1:8 boundary/folder-imports [advisory] specifier is not a literal",
            "ui/load.cjs:2:8 boundary/banned-imports [advisory] specifier is not a literal",
            "ui/load.cjs:2:8 boundary/folder-imports [advisory] specifier is not a literal",
            "ui/load.mjs:1:14 boundary/banned-imports [advisory] specifier is not a literal",
            "ui/load.mjs:1:14 boundary/folder-imports [advisory] specifier is not a literal",
        ]
    );
}

#[test]
fn type_only_query_uses_both_package_conditions() {
    let manifest = r##"{"imports":{"#choice":{"import":"./ok.mjs","require":"./db/x.cjs"},"#choice?raw":{"import":"./ok.mjs","require":"./db/x.cjs"}}}"##;
    let out = judge(
        &[
            ("blackhole.toml", FOLDERS),
            ("package.json", manifest),
            ("ui/a.ts", "import type T from '#choice?raw';"),
            ("db/x.cjs", ""),
            ("ok.mjs", ""),
        ],
        &[],
        0,
    );
    assert_eq!(
        lines_for(&out, "ui/a.ts"),
        ["ui/a.ts:1:20 boundary/folder-imports [advisory] db/x.cjs"]
    );
}

#[test]
fn imported_bindings_reexport_their_module() {
    let out = judge(
        &[
            ("blackhole.toml", FOLDERS),
            (
                "ui/app.mts",
                "import '../named.cts';\nimport '../direct.cts';\nimport '../assignment.cts';\nimport '../default.mts';\nimport '../namespace.mts';\nimport '../local.mts';",
            ),
            ("db/item.cjs", "module.exports = 1;"),
            (
                "named.cts",
                "import item = require('./db/item.cjs'); export { item };",
            ),
            (
                "direct.cts",
                "export import item = require('./db/item.cjs');",
            ),
            (
                "assignment.cts",
                "import item = require('./db/item.cjs'); export = item;",
            ),
            (
                "default.mts",
                "import item from './db/item.cjs'; export default item;",
            ),
            (
                "namespace.mts",
                "import * as item from './db/item.cjs'; export { item };",
            ),
            (
                "local.mts",
                "import item from './db/item.cjs'; const local = item; export { local };",
            ),
        ],
        &[],
        0,
    );
    assert_eq!(
        lines_for(&out, "ui/app.mts:"),
        [
            "ui/app.mts:1:8 boundary/folder-imports [advisory] db/item.cjs",
            "ui/app.mts:2:8 boundary/folder-imports [advisory] db/item.cjs",
            "ui/app.mts:3:8 boundary/folder-imports [advisory] db/item.cjs",
            "ui/app.mts:4:8 boundary/folder-imports [advisory] db/item.cjs",
            "ui/app.mts:5:8 boundary/folder-imports [advisory] db/item.cjs",
        ]
    );
}

#[test]
fn mixed_reexports_follow_the_exported_type_and_value_readings() {
    let out = judge(
        &[
            (
                "blackhole.toml",
                "[[boundaries.folders]]\nfrom='ui/**'\ndeny=['node_modules/pkg/db/**']",
            ),
            (
                "node_modules/pkg/package.json",
                r#"{"name":"pkg","exports":{".":{"types":"./db/secret.d.mts","import":"./public.mjs"},"./private":{"types":"./public.d.mts","import":"./db/private.mjs"}}}"#,
            ),
            (
                "node_modules/pkg/db/secret.d.mts",
                "export type Secret = string;",
            ),
            ("node_modules/pkg/public.mjs", "export const value = 1;"),
            (
                "node_modules/pkg/public.d.mts",
                "export type Public = string;",
            ),
            ("node_modules/pkg/db/private.mjs", "export const value = 1;"),
            (
                "ui/app.mts",
                "import './mixed.mts';\nimport './direct.mts';\nimport './value.mts';\nimport './private.mts';",
            ),
            (
                "ui/mixed.mts",
                "import { value, type Secret } from 'pkg'; export { Secret };",
            ),
            ("ui/direct.mts", "export { value, type Secret } from 'pkg';"),
            (
                "ui/value.mts",
                "import { value, type Secret } from 'pkg'; export { value };",
            ),
            (
                "ui/private.mts",
                "import { value, type Public } from 'pkg/private'; export { Public };",
            ),
        ],
        &[],
        0,
    );
    assert_eq!(
        lines_for(&out, "ui/"),
        [
            "ui/app.mts:1:8 boundary/folder-imports [advisory] node_modules/pkg/db/secret.d.mts",
            "ui/app.mts:2:8 boundary/folder-imports [advisory] node_modules/pkg/db/secret.d.mts",
            "ui/direct.mts:1:36 boundary/folder-imports [advisory] node_modules/pkg/db/secret.d.mts",
            "ui/mixed.mts:1:36 boundary/folder-imports [advisory] node_modules/pkg/db/secret.d.mts",
            "ui/private.mts:1:36 boundary/folder-imports [advisory] node_modules/pkg/db/private.mjs",
            "ui/value.mts:1:36 boundary/folder-imports [advisory] node_modules/pkg/db/secret.d.mts",
        ]
    );
}

#[test]
fn local_type_reexports_follow_the_declaration_reading() {
    let out = judge(
        &[
            ("blackhole.toml", FOLDERS),
            (
                "package.json",
                r##"{"imports":{"#model":{"types":"./db/secret.d.mts","default":"./public.mjs"}}}"##,
            ),
            ("db/secret.d.mts", "export type Secret = string;"),
            ("public.mjs", "export const Secret = 1;"),
            (
                "ui/app.mts",
                "import '../declaration.mts';\nimport '../specifier.mts';\nimport '../type-import.mts';\nimport '../direct.mts';\nimport '../value.mts';",
            ),
            (
                "declaration.mts",
                "import { Secret } from '#model'; export type { Secret };",
            ),
            (
                "specifier.mts",
                "import { Secret } from '#model'; export { type Secret };",
            ),
            (
                "type-import.mts",
                "import type { Secret } from '#model'; export { Secret };",
            ),
            ("direct.mts", "export type { Secret } from '#model';"),
            (
                "value.mts",
                "import { Secret } from '#model'; export { Secret };",
            ),
        ],
        &[],
        0,
    );
    assert_eq!(
        lines_for(&out, "ui/app.mts:"),
        [
            "ui/app.mts:1:8 boundary/folder-imports [advisory] db/secret.d.mts",
            "ui/app.mts:2:8 boundary/folder-imports [advisory] db/secret.d.mts",
            "ui/app.mts:3:8 boundary/folder-imports [advisory] db/secret.d.mts",
            "ui/app.mts:4:8 boundary/folder-imports [advisory] db/secret.d.mts",
        ]
    );
}

fn import_equals_reexports(manifest: &str) -> String {
    judge(
        &[
            ("blackhole.toml", FOLDERS),
            ("package.json", manifest),
            ("db/secret.d.mts", "export type Secret = string;"),
            ("public.d.mts", "export type Secret = string;"),
            ("db/secret.cjs", "module.exports = 1;"),
            ("public.cjs", "module.exports = 1;"),
            (
                "ui/app.cts",
                "import '../declaration.cjs';\nimport '../specifier.cjs';\nimport '../value.cjs';\nimport '../export-import.cjs';\nimport '../assignment.cjs';\nimport '../both.cjs';",
            ),
            (
                "declaration.cts",
                "import model = require('#model'); export type { model as M };",
            ),
            (
                "specifier.cts",
                "import model = require('#model'); export { type model as M };",
            ),
            (
                "value.cts",
                "import model = require('#model'); export { model };",
            ),
            (
                "export-import.cts",
                "export import model = require('#model');",
            ),
            (
                "assignment.cts",
                "import model = require('#model'); export = model;",
            ),
            (
                "both.cts",
                "import model = require('#model'); export { model, type model as M };",
            ),
        ],
        &[],
        0,
    )
}

#[test]
fn import_equals_reexports_follow_the_declaration_when_exported_as_a_type() {
    let out = import_equals_reexports(
        r##"{"imports":{"#model":{"types":"./db/secret.d.mts","default":"./public.cjs"}}}"##,
    );
    assert_eq!(
        lines_for(&out, "ui/app.cts:"),
        [
            "ui/app.cts:1:8 boundary/folder-imports [advisory] db/secret.d.mts",
            "ui/app.cts:2:8 boundary/folder-imports [advisory] db/secret.d.mts",
            "ui/app.cts:6:8 boundary/folder-imports [advisory] db/secret.d.mts",
        ]
    );
}

#[test]
fn import_equals_reexports_follow_the_runtime_when_exported_as_a_value() {
    let out = import_equals_reexports(
        r##"{"imports":{"#model":{"types":"./public.d.mts","default":"./db/secret.cjs"}}}"##,
    );
    assert_eq!(
        lines_for(&out, "ui/app.cts:"),
        [
            "ui/app.cts:3:8 boundary/folder-imports [advisory] db/secret.cjs",
            "ui/app.cts:4:8 boundary/folder-imports [advisory] db/secret.cjs",
            "ui/app.cts:5:8 boundary/folder-imports [advisory] db/secret.cjs",
            "ui/app.cts:6:8 boundary/folder-imports [advisory] db/secret.cjs",
        ]
    );
}

#[test]
fn module_sync_is_available_to_both_runtime_loaders() {
    let out = judge(
        &[
            (
                "blackhole.toml",
                "[[boundaries.folders]]\nfrom='ui/**'\ndeny=['node_modules/**/db/**']",
            ),
            (
                "node_modules/modern/package.json",
                r#"{"name":"modern","exports":{"module-sync":"./db/sync.mjs","node-addons":"./db/addons.cjs","import":"./public.mjs","require":"./public.cjs"}}"#,
            ),
            ("node_modules/modern/db/sync.mjs", "export {};"),
            ("node_modules/modern/db/addons.cjs", ""),
            (
                "node_modules/legacy/package.json",
                r#"{"name":"legacy","exports":{"node-addons":"./db/addons.cjs","import":"./public.mjs","require":"./public.cjs"}}"#,
            ),
            ("node_modules/legacy/db/addons.cjs", ""),
            ("ui/esm.mts", "import 'modern';\nimport 'legacy';"),
            ("ui/cjs.cts", "require('modern');\nrequire('legacy');"),
        ],
        &[],
        0,
    );
    assert_eq!(
        lines_for(&out, "ui/"),
        [
            "ui/cjs.cts:1:9 boundary/folder-imports [advisory] node_modules/modern/db/sync.mjs",
            "ui/cjs.cts:2:9 boundary/folder-imports [advisory] node_modules/legacy/db/addons.cjs",
            "ui/esm.mts:1:8 boundary/folder-imports [advisory] node_modules/modern/db/sync.mjs",
            "ui/esm.mts:2:8 boundary/folder-imports [advisory] node_modules/legacy/db/addons.cjs",
        ]
    );
}

#[test]
fn javascript_folder_checks_follow_the_javascript_reexport() {
    let out = judge(
        &[
            ("blackhole.toml", FOLDERS),
            ("ui/a.mjs", "import '../public.js';"),
            ("public.js", "export * from './db/model.js';"),
            ("public.ts", ""),
            ("db/model.js", ""),
        ],
        &[],
        0,
    );
    assert!(out.contains("ui/a.mjs:1:8 boundary/folder-imports [advisory] db/model.js"));
}

#[test]
fn javascript_package_import_alias_uses_javascript_when_typescript_is_adjacent() {
    for (denied, code) in [("db/model.js", 1), ("db/model.ts", 0)] {
        let out = judge(
            &[
                (
                    "blackhole.toml",
                    &format!("[[boundaries.folders]]\nfrom='ui/**'\ndeny=['{denied}']"),
                ),
                (
                    "package.json",
                    r##"{"type":"module","imports":{"#model":"./db/model.js"}}"##,
                ),
                ("ui/app.js", "import '#model';"),
                ("db/model.js", ""),
                ("db/model.ts", ""),
            ],
            &[],
            0,
        );
        assert_eq!(
            out.contains("boundary/folder-imports [advisory] db/model.js"),
            code == 1,
            "{out}"
        );
    }
}

fn type_import_follows_declaration_reexports(extension: &str, declaration: &str) {
    let out = judge(
        &[
            ("blackhole.toml", FOLDERS),
            (
                "ui/a.ts",
                &format!("import type {{Model}} from '../public.{extension}';"),
            ),
            (&format!("public.{extension}"), ""),
            (
                &format!("public.{declaration}"),
                &format!("export type {{Model}} from './db/model.{extension}';"),
            ),
            (
                &format!("db/model.{declaration}"),
                "export interface Model {}",
            ),
        ],
        &[],
        0,
    );
    assert!(
        out.contains(&format!(
            "boundary/folder-imports [advisory] db/model.{declaration}"
        )),
        "{out}"
    );
}

#[test]
fn type_import_follows_plain_barrel_reexports_as_types() {
    let notes = [
        ("node_modules/sample/model.d.mts", "ui/app.mts:1:26"),
        ("node_modules/sample/model.mjs", "ui/app.mts:2:37"),
    ]
    .map(|(denied, position)| {
        let out = judge(
            &[
                (
                    "blackhole.toml",
                    &format!("[[boundaries.folders]]\nfrom='ui/**'\ndeny=['{denied}']"),
                ),
                (
                    "ui/app.mts",
                    "import type {Model} from '../barrel.mjs';\nimport {Model as RuntimeModel} from '../barrel.mjs';",
                ),
                ("barrel.mts", "export {Model} from 'sample';"),
                (
                    "node_modules/sample/package.json",
                    r#"{"exports":{"types":"./model.d.mts","default":"./model.mjs"}}"#,
                ),
                ("node_modules/sample/model.d.mts", "export interface Model {}"),
                ("node_modules/sample/model.mjs", "export const Model = {};"),
            ],
            &[],
            0,
        );
        (
            lines_for(&out, "ui/app.mts:")
                .into_iter()
                .map(str::to_owned)
                .collect::<Vec<_>>(),
            vec![format!("{position} boundary/folder-imports [advisory] {denied}")],
        )
    });
    let [(types, expected_types), (runtime, expected_runtime)] = notes;
    assert_eq!([types, runtime], [expected_types, expected_runtime]);
}

#[test]
fn mjs_type_import_prefers_the_d_mts_file() {
    type_import_follows_declaration_reexports("mjs", "d.mts");
}

#[test]
fn cjs_type_import_prefers_the_d_cts_file() {
    type_import_follows_declaration_reexports("cjs", "d.cts");
}

#[test]
fn js_type_import_prefers_the_d_ts_file() {
    type_import_follows_declaration_reexports("js", "d.ts");
}

#[test]
fn declaration_import_resolves_the_declaration_for_a_javascript_specifier() {
    let out = judge(
        &[
            (
                "blackhole.toml",
                "[[boundaries.folders]]\nfrom='ui/**'\ndeny=['db/model.d.ts']",
            ),
            (
                "ui/app.d.ts",
                "import {Model} from '../db/model.js'; export type App = Model;",
            ),
            ("db/model.d.ts", "export interface Model {}"),
        ],
        &[],
        0,
    );
    assert!(
        out.contains("boundary/folder-imports [advisory] db/model.d.ts"),
        "{out}"
    );
}

#[test]
fn extension_only_javascript_importers_use_javascript() {
    let out = judge(
        &[
            ("blackhole.toml", FOLDERS),
            ("ui/.mjs", "import '../db/target.js';"),
            ("db/target.js", ""),
            ("db/target.ts", ""),
        ],
        &[],
        0,
    );
    assert!(out.contains("ui/.mjs:1:8 boundary/folder-imports [advisory] db/target.js"));
    assert!(!out.contains("db/target.ts"));
}

#[test]
fn extensionless_type_imports_prefer_declarations_over_javascript() {
    let out = judge(
        &[
            ("blackhole.toml", FOLDERS),
            ("ui/app.cts", "import type {Model} from '../public';"),
            ("public.d.ts", "export type {Model} from './db/model';"),
            ("public.js", ""),
            ("db/model.d.ts", "export interface Model {}"),
        ],
        &[],
        0,
    );
    assert!(out.contains("boundary/folder-imports [advisory] db/model.d.ts"));
}

const TYPE_PACKAGE: &str = "import type {Model} from 'demo';";
const PACKAGE_MODEL: &str = "node_modules/demo/db/model.d.ts";
const PACKAGE_INDEX: &str = "node_modules/demo/index.d.ts";
const PACKAGE_REEXPORT: &str = "export type {Model} from './db/model.js';";
const PACKAGE_PUBLIC: &str = "export interface Model {}";

fn package_declarations(
    importer: &str,
    source: &str,
    manifest: &str,
    declarations: &[(&str, &str)],
    denied: Option<&str>,
) {
    let mut files = vec![
        (
            "blackhole.toml",
            "[[boundaries.folders]]\nfrom='ui/**'\ndeny=['node_modules/demo/db/**']",
        ),
        (importer, source),
        ("node_modules/demo/package.json", manifest),
        ("node_modules/demo/entry.js", ""),
        (
            "node_modules/demo/db/model.d.ts",
            "export interface Model {}",
        ),
    ];
    files.extend(declarations);
    let out = judge(&files, &[], 0);
    let findings: Vec<_> = lines_for(&out, importer)
        .into_iter()
        .map(|line| {
            line.split_once(" boundary/folder-imports [advisory] ")
                .unwrap()
                .1
        })
        .collect();
    assert_eq!(findings, denied.into_iter().collect::<Vec<_>>());
}

#[test]
fn type_package_imports_bare_target_uses_index_declaration() {
    let out = judge(
        &[
            (
                "blackhole.toml",
                "[[boundaries.folders]]\nfrom='ui/**'\ndeny=['node_modules/demo/index.d.ts']",
            ),
            ("ui/package.json", r##"{"imports":{"#model":"demo"}}"##),
            ("ui/app.cts", "import type {Model} from '#model';"),
            ("node_modules/demo/package.json", r#"{"main":"./entry.js"}"#),
            ("node_modules/demo/entry.js", ""),
            (PACKAGE_INDEX, PACKAGE_PUBLIC),
        ],
        &[],
        0,
    );
    assert_eq!(
        lines_for(&out, "ui/app.cts"),
        ["ui/app.cts:1:26 boundary/folder-imports [advisory] node_modules/demo/index.d.ts"]
    );
}

#[test]
fn type_package_imports_uses_first_matching_versioned_condition() {
    let out = judge(
        &[
            ("blackhole.toml", FOLDERS),
            (
                "package.json",
                r##"{"type":"module","imports":{"#model":{"types@>=6":"./db/other.d.ts","types@>=5.9":"./db/model.d.ts","types":"./allowed.d.ts"}}}"##,
            ),
            ("ui/app.ts", "import type {Model} from '#model';"),
            ("db/other.d.ts", PACKAGE_PUBLIC),
            ("db/model.d.ts", PACKAGE_PUBLIC),
            ("allowed.d.ts", PACKAGE_PUBLIC),
        ],
        &[],
        0,
    );
    assert_eq!(
        lines_for(&out, "ui/app.ts"),
        ["ui/app.ts:1:26 boundary/folder-imports [advisory] db/model.d.ts"]
    );
}

#[test]
fn type_package_imports_continues_after_missing_bare_target() {
    let out = judge(
        &[
            (
                "blackhole.toml",
                "[[boundaries.folders]]\nfrom='ui/**'\ndeny=['node_modules/demo/index.d.ts']",
            ),
            (
                "ui/package.json",
                r##"{"imports":{"#model":{"types":"missing-package","default":"demo"}}}"##,
            ),
            ("ui/app.cts", "import type {Model} from '#model';"),
            (
                "node_modules/demo/package.json",
                r#"{"types":"./index.d.ts"}"#,
            ),
            (PACKAGE_INDEX, PACKAGE_PUBLIC),
        ],
        &[],
        0,
    );
    assert_eq!(
        lines_for(&out, "ui/app.cts"),
        ["ui/app.cts:1:26 boundary/folder-imports [advisory] node_modules/demo/index.d.ts"]
    );
}

#[test]
fn type_package_imports_yields_to_paths_mapping() {
    let out = judge(
        &[
            ("blackhole.toml", FOLDERS),
            (
                "tsconfig.json",
                r##"{"compilerOptions":{"paths":{"#model":["./db/model.d.ts"]}}}"##,
            ),
            (
                "package.json",
                r##"{"imports":{"#model":"./allowed.d.ts"}}"##,
            ),
            ("ui/app.cts", "import type {Model} from '#model';"),
            ("db/model.d.ts", PACKAGE_PUBLIC),
            ("allowed.d.ts", PACKAGE_PUBLIC),
        ],
        &[],
        0,
    );
    assert_eq!(
        lines_for(&out, "ui/app.cts"),
        ["ui/app.cts:1:26 boundary/folder-imports [advisory] db/model.d.ts"]
    );
}

#[test]
fn type_package_imports_continues_after_missing_versioned_target() {
    let out = judge(
        &[
            ("blackhole.toml", FOLDERS),
            (
                "package.json",
                r##"{"imports":{"#model":{"types@>=5.9":"./missing.d.ts","types":"./db/model.d.ts"}}}"##,
            ),
            ("ui/app.mts", "import type {Model} from '#model';"),
            ("db/model.d.ts", PACKAGE_PUBLIC),
        ],
        &[],
        0,
    );
    assert_eq!(
        lines_for(&out, "ui/app.mts"),
        ["ui/app.mts:1:26 boundary/folder-imports [advisory] db/model.d.ts"]
    );
}

#[test]
fn type_package_manifest_accepts_utf8_bom() {
    package_declarations(
        "ui/app.cts",
        TYPE_PACKAGE,
        concat!("\u{feff}", r#"{"types":"./public.d.ts"}"#),
        &[
            ("node_modules/demo/public.d.ts", PACKAGE_PUBLIC),
            (PACKAGE_INDEX, PACKAGE_REEXPORT),
        ],
        None,
    );
}

#[test]
fn type_package_java_script_main_uses_index_declaration() {
    package_declarations(
        "ui/app.cts",
        TYPE_PACKAGE,
        r#"{"main":"./entry.js"}"#,
        &[(PACKAGE_INDEX, PACKAGE_REEXPORT)],
        Some(PACKAGE_MODEL),
    );
}

#[test]
fn type_package_extensionless_main_uses_index_declaration() {
    package_declarations(
        "ui/app.cts",
        TYPE_PACKAGE,
        r#"{"main":"./entry"}"#,
        &[(PACKAGE_INDEX, PACKAGE_REEXPORT)],
        Some(PACKAGE_MODEL),
    );
}

#[test]
fn type_package_main_substitutes_declaration_before_index() {
    package_declarations(
        "ui/app.cts",
        TYPE_PACKAGE,
        r#"{"main":"./entry.js"}"#,
        &[
            (PACKAGE_INDEX, PACKAGE_PUBLIC),
            ("node_modules/demo/entry.d.ts", PACKAGE_REEXPORT),
        ],
        Some(PACKAGE_MODEL),
    );
}

#[test]
fn type_package_typings_precedes_main() {
    package_declarations(
        "ui/app.cts",
        TYPE_PACKAGE,
        r#"{"main":"./entry.js","typings":"./index.d.ts"}"#,
        &[(PACKAGE_INDEX, PACKAGE_REEXPORT)],
        Some(PACKAGE_MODEL),
    );
}

#[test]
fn type_package_versions_remaps_types() {
    package_declarations(
        "ui/app.cts",
        TYPE_PACKAGE,
        r#"{"types":"./index.d.ts","typesVersions":{"*":{"index.d.ts":["db/index.d.ts"]}}}"#,
        &[
            (PACKAGE_INDEX, PACKAGE_PUBLIC),
            ("node_modules/demo/db/index.d.ts", PACKAGE_PUBLIC),
        ],
        Some("node_modules/demo/db/index.d.ts"),
    );
}

#[test]
fn type_package_import_versions_miss_uses_package_index() {
    package_declarations(
        "ui/app.mts",
        TYPE_PACKAGE,
        r#"{"type":"module","types":"./index.d.ts","typesVersions":{"*":{"index.d.ts":["db/model"]}}}"#,
        &[(PACKAGE_INDEX, PACKAGE_REEXPORT)],
        Some(PACKAGE_MODEL),
    );
}

#[test]
fn type_package_versions_remaps_subpaths() {
    package_declarations(
        "ui/app.cts",
        "import type {Model} from 'demo/sub';",
        r#"{"types":"./index.d.ts","typesVersions":{">=5.4 <6":{"*":["db/*"]},"*":{"*":["*"]}}}"#,
        &[
            (PACKAGE_INDEX, PACKAGE_PUBLIC),
            ("node_modules/demo/sub.d.ts", PACKAGE_PUBLIC),
            ("node_modules/demo/db/sub.d.ts", PACKAGE_PUBLIC),
        ],
        Some("node_modules/demo/db/sub.d.ts"),
    );
}

#[test]
fn type_package_nonmatching_versions_uses_types() {
    package_declarations(
        "ui/app.cts",
        TYPE_PACKAGE,
        r#"{"types":"./index.d.ts","typesVersions":{"<5":{"index.d.ts":["db/model.d.ts"]}}}"#,
        &[(PACKAGE_INDEX, PACKAGE_PUBLIC)],
        None,
    );
}

#[test]
fn type_package_versions_checks_integer_keys_before_string_keys() {
    package_declarations(
        "ui/app.cts",
        TYPE_PACKAGE,
        r#"{"types":"./index.d.ts","typesVersions":{"*":{"index.d.ts":["index.d.ts"]},"5":{"index.d.ts":["db/model.d.ts"]}}}"#,
        &[(PACKAGE_INDEX, PACKAGE_PUBLIC)],
        Some(PACKAGE_MODEL),
    );
}

#[test]
fn type_package_exports_shadows_versions() {
    package_declarations(
        "ui/app.mts",
        TYPE_PACKAGE,
        r#"{"exports":{"types":"./index.d.ts","import":"./entry.js"},"types":"./index.d.ts","typesVersions":{"*":{"index.d.ts":["db/model.d.ts"]}}}"#,
        &[(PACKAGE_INDEX, PACKAGE_PUBLIC)],
        None,
    );
}

#[test]
fn type_package_without_declarations_uses_javascript_main() {
    package_declarations(
        "ui/app.cts",
        TYPE_PACKAGE,
        r#"{"main":"./db/entry.js"}"#,
        &[("node_modules/demo/db/entry.js", "")],
        Some("node_modules/demo/db/entry.js"),
    );
}

#[test]
fn type_package_declaration_keeps_denied_javascript_main_clean() {
    package_declarations(
        "ui/app.cts",
        TYPE_PACKAGE,
        r#"{"main":"./db/entry.js"}"#,
        &[
            ("node_modules/demo/db/entry.js", ""),
            (PACKAGE_INDEX, PACKAGE_PUBLIC),
        ],
        None,
    );
}

#[test]
fn type_package_javascript_fallback_uses_type_mode_conditions() {
    package_declarations(
        "ui/app.mts",
        TYPE_PACKAGE,
        r#"{"exports":{"types":"./missing.d.ts","module-sync":"./entry.js","import":"./db/entry.mjs"}}"#,
        &[("node_modules/demo/db/entry.mjs", "")],
        Some("node_modules/demo/db/entry.mjs"),
    );
}

#[test]
fn type_package_import_skips_legacy_subpath_index() {
    package_declarations(
        "ui/app.mts",
        "import type {Model} from 'demo/db';",
        "{}",
        &[
            ("node_modules/demo/db/index.d.ts", PACKAGE_PUBLIC),
            ("node_modules/demo/db/index.js", ""),
        ],
        None,
    );
}

#[test]
fn type_package_require_uses_legacy_subpath_index() {
    package_declarations(
        "ui/app.cts",
        "import type {Model} from 'demo/db';",
        "{}",
        &[
            ("node_modules/demo/db/index.d.ts", PACKAGE_PUBLIC),
            ("node_modules/demo/db/index.js", ""),
        ],
        Some("node_modules/demo/db/index.d.ts"),
    );
}

#[test]
fn type_package_require_uses_index_declaration() {
    package_declarations(
        "ui/app.cts",
        "type Model = import('demo', {with: {'resolution-mode': 'require'}}).Model;",
        r#"{"main":"./entry.js"}"#,
        &[(PACKAGE_INDEX, PACKAGE_REEXPORT)],
        Some(PACKAGE_MODEL),
    );
}

#[test]
fn type_package_exports_uses_manifest_order() {
    package_declarations(
        "ui/app.mts",
        TYPE_PACKAGE,
        r#"{"exports":{"default":"./entry.js","types":"./index.d.ts"}}"#,
        &[
            (PACKAGE_INDEX, PACKAGE_PUBLIC),
            ("node_modules/demo/entry.d.ts", PACKAGE_REEXPORT),
        ],
        Some(PACKAGE_MODEL),
    );
}

#[test]
fn type_local_file_precedes_directory_declaration() {
    let out = judge(
        &[
            (
                "blackhole.toml",
                "[[boundaries.folders]]\nfrom='ui/**'\ndeny=['ui/foo.js','ui/foo/**']",
            ),
            ("ui/app.cts", "import type {Model} from './foo';"),
            ("ui/foo.js", ""),
            ("ui/foo/index.d.ts", PACKAGE_PUBLIC),
        ],
        &[],
        0,
    );
    assert_eq!(
        lines_for(&out, "ui/app.cts"),
        ["ui/app.cts:1:26 boundary/folder-imports [advisory] ui/foo.js"]
    );
}

#[test]
fn type_local_directory_prefers_typings_over_types() {
    let out = judge(
        &[
            (
                "blackhole.toml",
                "[[boundaries.folders]]\nfrom='ui/**'\ndeny=['ui/lib/**']",
            ),
            ("ui/app.cts", "import type {Model} from './lib';"),
            (
                "ui/lib/package.json",
                r#"{"types":"./types.d.ts","typings":"./typings.d.ts"}"#,
            ),
            ("ui/lib/types.d.ts", PACKAGE_PUBLIC),
            ("ui/lib/typings.d.ts", PACKAGE_PUBLIC),
        ],
        &[],
        0,
    );
    assert_eq!(
        lines_for(&out, "ui/app.cts"),
        ["ui/app.cts:1:26 boundary/folder-imports [advisory] ui/lib/typings.d.ts"]
    );
}

#[test]
fn type_paths_file_precedes_directory_declaration() {
    let out = judge(
        &[
            (
                "blackhole.toml",
                "[[boundaries.folders]]\nfrom='ui/**'\ndeny=['alias/**']",
            ),
            (
                "tsconfig.json",
                r#"{"compilerOptions":{"paths":{"aliased/*":["./alias/*"]}}}"#,
            ),
            ("ui/app.cts", "import type {Model} from 'aliased/x';"),
            ("alias/x.js", ""),
            ("alias/x/index.d.ts", PACKAGE_PUBLIC),
        ],
        &[],
        0,
    );
    assert_eq!(
        lines_for(&out, "ui/app.cts"),
        ["ui/app.cts:1:26 boundary/folder-imports [advisory] alias/x.js"]
    );
}

#[test]
fn type_local_jsx_resolves_to_ts() {
    let out = judge(
        &[
            (
                "blackhole.toml",
                "[[boundaries.folders]]\nfrom='ui/**'\ndeny=['ui/x.ts']",
            ),
            ("ui/app.cts", "import type {Model} from './x.jsx';"),
            ("ui/x.ts", PACKAGE_PUBLIC),
        ],
        &[],
        0,
    );
    assert_eq!(
        lines_for(&out, "ui/app.cts"),
        ["ui/app.cts:1:26 boundary/folder-imports [advisory] ui/x.ts"]
    );
}

#[test]
fn type_local_extensionless_path_skips_mts() {
    let out = judge(
        &[
            (
                "blackhole.toml",
                "[[boundaries.folders]]\nfrom='ui/**'\ndeny=['ui/only-mts/foo.mts']",
            ),
            ("ui/app.cts", "import type {Model} from './only-mts/foo';"),
            ("ui/only-mts/foo.mts", PACKAGE_PUBLIC),
        ],
        &[],
        0,
    );
    assert_eq!(lines_for(&out, "ui/app.cts"), [] as [&str; 0]);
}

#[test]
fn type_local_extensionless_path_skips_mjs_and_cjs() {
    for extension in ["mjs", "cjs"] {
        let out = judge(
            &[
                (
                    "blackhole.toml",
                    "[[boundaries.folders]]\nfrom='ui/**'\ndeny=['ui/model/**']",
                ),
                ("ui/package.json", r#"{"type":"commonjs"}"#),
                ("ui/app.ts", "import type {Model} from './model';"),
                (&format!("ui/model.{extension}"), ""),
                ("ui/model/index.d.ts", PACKAGE_PUBLIC),
            ],
            &[],
            0,
        );
        assert_eq!(
            lines_for(&out, "ui/app.ts"),
            ["ui/app.ts:1:26 boundary/folder-imports [advisory] ui/model/index.d.ts"]
        );
    }
}

#[test]
fn type_package_versions_ignores_empty_alternative() {
    package_declarations(
        "ui/app.cts",
        TYPE_PACKAGE,
        r#"{"types":"./index.d.ts","typesVersions":{"<5 || ":{"index.d.ts":["db/model.d.ts"]}}}"#,
        &[(PACKAGE_INDEX, PACKAGE_PUBLIC)],
        None,
    );
}

#[test]
fn type_package_question_mark_uses_literal_file() {
    let out = judge(
        &[
            (
                "blackhole.toml",
                "[[boundaries.folders]]\nfrom='ui/**'\ndeny=['node_modules/demo/sub*']",
            ),
            ("ui/app.cts", "import type {Model} from 'demo/sub?x';"),
            ("node_modules/demo/package.json", "{}"),
            ("node_modules/demo/sub?x", ""),
        ],
        &[],
        0,
    );
    assert_eq!(
        lines_for(&out, "ui/app.cts"),
        ["ui/app.cts:1:26 boundary/folder-imports [advisory] node_modules/demo/sub?x"]
    );
}

#[test]
fn type_package_deep_javascript_fallback_stays_within_read_budget() {
    let mut dir = "ui".to_owned();
    let mut manifests = vec!["node_modules/demo/package.json".to_owned()];
    for _ in 0..33 {
        manifests.push(format!("{dir}/node_modules/demo/package.json"));
        dir.push_str("/deep");
    }
    let importer = format!("{dir}/app.cts");
    let mut files = vec![
        (
            "blackhole.toml",
            "[[boundaries.folders]]\nfrom='ui/**'\ndeny=['node_modules/demo/**']",
        ),
        (&importer, TYPE_PACKAGE),
        ("node_modules/demo/index.js", ""),
    ];
    files.extend(manifests.iter().map(|path| (path.as_str(), "{}")));
    let out = judge(&files, &[], 0);
    assert_eq!(
        lines_for(&out, &importer),
        [format!(
            "{importer}:1:26 boundary/folder-imports [advisory] node_modules/demo/index.js"
        )]
    );
}

#[test]
fn type_package_versions_selects_matching_ranges() {
    for (matching, nonmatching) in [
        ("*", None),
        ("5.*", Some("4.*")),
        ("5.x", Some("4.x")),
        ("5.X", Some("4.X")),
        ("5.9", Some("5.8")),
        ("<5.10", Some("<5.9")),
        ("<=5.9", Some("<=5.9.2")),
        (">5.8", Some(">5.9")),
        (">=5.9", Some(">=5.10")),
        ("=5.9.3", Some("=5.9.2")),
        ("^5.8", Some("^4.9")),
        ("~5.9.2", Some("~5.8")),
        ("5.8 - 5.9", Some("5.7 - 5.8")),
        ("<5 || >=5.9", Some("<5 || >=6")),
        (">=5.9.3-rc.1", Some("<5.9.3-rc.1")),
        ("5.9.3+build.1", Some("5.9+build.1")),
        ("", None),
    ] {
        let cases = [(matching, Some(PACKAGE_MODEL))]
            .into_iter()
            .chain(nonmatching.map(|range| (range, None)));
        for (range, denied) in cases {
            package_declarations(
                "ui/app.cts",
                TYPE_PACKAGE,
                &format!(
                    r#"{{"types":"./index.d.ts","typesVersions":{{"{range}":{{"index.d.ts":["db/model.d.ts"]}}}}}}"#
                ),
                &[(PACKAGE_INDEX, PACKAGE_PUBLIC)],
                denied,
            );
        }
    }
}

#[test]
fn type_package_exports_prefers_specific_star_pattern() {
    package_declarations(
        "ui/app.cts",
        "import type {Model} from 'demo/sub/model.js';",
        r#"{"exports":{"./*":"./index.d.ts","./sub/*":"./index.d.ts","./sub/*.js":"./db/*.d.ts"}}"#,
        &[(PACKAGE_INDEX, PACKAGE_PUBLIC)],
        Some(PACKAGE_MODEL),
    );
}

#[test]
fn type_package_exports_maps_trailing_slash_folder() {
    package_declarations(
        "ui/app.cts",
        "import type {Model} from 'demo/sub/model.d.ts';",
        r#"{"exports":{"./sub/":"./db/"}}"#,
        &[],
        Some(PACKAGE_MODEL),
    );
}

#[test]
fn type_package_exports_uses_first_resolvable_array_target() {
    package_declarations(
        "ui/app.cts",
        "import type {Model} from 'demo/sub';",
        r#"{"exports":{"./sub":["./missing.d.ts","./db/model.d.ts","./index.d.ts"]}}"#,
        &[(PACKAGE_INDEX, PACKAGE_PUBLIC)],
        Some(PACKAGE_MODEL),
    );
}

#[test]
fn type_package_exports_array_continues_past_invalid_targets() {
    for (fallback, finding) in [
        ("./index.d.ts", None),
        (
            "./missing.d.ts",
            Some(
                r#"ui/app.mts:1:26 boundary/folder-imports [advisory] invalid package export target "invalid" in /node_modules/demo"#,
            ),
        ),
    ] {
        let manifest = format!(r#"{{"exports":["invalid","{fallback}"]}}"#);
        let out = judge(
            &[
                ("blackhole.toml", FOLDERS),
                ("ui/app.mts", TYPE_PACKAGE),
                ("node_modules/demo/package.json", &manifest),
                (PACKAGE_INDEX, PACKAGE_PUBLIC),
            ],
            &[],
            0,
        );
        assert_eq!(
            lines_for(&out, "ui/app.mts"),
            finding.into_iter().collect::<Vec<_>>()
        );
    }
}

#[test]
fn type_package_exports_array_can_fall_back_to_javascript_after_invalid_target() {
    let out = judge(
        &[
            (
                "blackhole.toml",
                "[[boundaries.folders]]\nfrom='ui/**'\ndeny=['node_modules/demo/index.js']",
            ),
            ("ui/app.mts", TYPE_PACKAGE),
            (
                "node_modules/demo/package.json",
                r#"{"exports":["invalid","./index.js"]}"#,
            ),
            ("node_modules/demo/index.js", ""),
        ],
        &[],
        0,
    );
    assert_eq!(
        lines_for(&out, "ui/app.mts"),
        ["ui/app.mts:1:26 boundary/folder-imports [advisory] node_modules/demo/index.js"]
    );
}

#[test]
fn type_package_exports_rejects_invalid_targets() {
    for target in ["../escape.d.ts", "./../escape.d.ts"] {
        let manifest = format!(r#"{{"exports":{{"./sub":"{target}"}}}}"#);
        let out = judge(
            &[
                ("blackhole.toml", FOLDERS),
                ("ui/app.cts", "import type {Model} from 'demo/sub';"),
                ("node_modules/demo/package.json", &manifest),
                ("node_modules/escape.d.ts", PACKAGE_PUBLIC),
            ],
            &[],
            0,
        );
        assert_eq!(
            lines_for(&out, "ui/app.cts"),
            [
                r#"ui/app.cts:1:26 boundary/folder-imports [advisory] invalid package export target "../escape.d.ts" in /node_modules/demo"#
            ],
        );
    }
}

const LOADS: [&str; 3] = [
    "module.constructor.createRequire(__filename)('./x.cjs');",
    "module.constructor.createRequire(import.meta.url)('./x.cjs');",
    "require('./x.cjs');",
];

#[test]
fn local_filename_preamble_preserves_meta_factory() {
    let source = "import { createRequire } from 'node:module';\nimport { fileURLToPath } from 'node:url';\nconst __filename = fileURLToPath(import.meta.url);\nconst require = createRequire(import.meta.url);\nrequire('./x.cjs');";
    assert_eq!(
        judge(&[("blackhole.toml", FOLDERS), ("ui/a.mjs", source)], &[], 0),
        ""
    );
}

#[test]
fn distrust_events_cover_each_loader_in_either_order() {
    for (event, mention) in [
        ("__filename = 'elsewhere';", Some("__filename")),
        ("eval('changeBase()');", None),
        ("arguments[3] = 'elsewhere';", Some("arguments")),
        (
            "module.filename = '/db/index.cjs';",
            Some("module.filename"),
        ),
        ("import.meta.url = 'elsewhere';", Some("import.meta")),
        ("delete import.meta.url;", Some("import.meta")),
        ("delete import.meta?.url;", Some("import.meta")),
        ("Object.assign(import.meta, {});", Some("import.meta")),
        ("globalThis.__filename = 'elsewhere';", Some("globalThis")),
        ("const key = '__filename';", None),
    ] {
        for load in LOADS {
            let file = if load.contains("import.meta") || event.contains("import.meta") {
                "ui/a.mjs"
            } else {
                "ui/a.cjs"
            };
            for (source, load_line, event_line) in [
                (format!("{event}\n{load}"), 2, 1),
                (format!("{load}\n{event}"), 1, 2),
                (format!("{load}\nconst never = () => {{ {event} }};"), 1, 2),
            ] {
                let mut expected = Vec::new();
                if let Some(mention) = mention {
                    let col = source
                        .lines()
                        .nth(event_line - 1)
                        .unwrap()
                        .find(mention)
                        .unwrap()
                        + 1;
                    expected.push(format!("{file}:{event_line}:{col} boundary/folder-imports [advisory] specifier is not a literal"));
                }
                if load.starts_with("module.constructor") {
                    expected.push(format!(
                        "{file}:{load_line}:1 boundary/folder-imports [advisory] specifier is not a literal"
                    ));
                }
                expected.sort();
                let out = judge(&[("blackhole.toml", FOLDERS), (file, &source)], &[], 0);
                assert_eq!(
                    out.lines()
                        .filter(|line| line.starts_with(file))
                        .collect::<Vec<_>>(),
                    expected,
                    "{source}"
                );
            }
        }
    }
}

#[test]
fn empty_boundaries_are_rejected() {
    for policy in [
        "[[boundaries.folders]]\nfrom=''\ndeny=['db/**']",
        "[[boundaries.folders]]\nfrom='ui/**'\ndeny=[]",
        "[[boundaries.folders]]\nfrom='ui/**'\ndeny=['']",
    ] {
        let out = judge(&[("blackhole.toml", policy)], &[], 2);
        assert!(out.contains("empty folder boundary"), "{out}");
    }
}

#[test]
fn unapproved_removal_keeps_the_base_folder_policy() {
    let dir = super::cli::repo();
    let root = dir.path();
    let git = |args: &[&str]| super::common::git(root, args).unwrap();
    super::put(root, "blackhole.toml", FOLDERS);
    super::put(root, "ui/a.ts", "import '../db/x';");
    git(&["add", "."]);
    git(&["commit", "-qm", "base"]);
    super::put(root, "blackhole.toml", "");
    git(&["commit", "-qam", "remove policy"]);

    let out = super::cli::run(root, &["ci", "--base", "HEAD^", "--head", "HEAD"], 1);
    let out = String::from_utf8(out.stdout).unwrap();
    assert_eq!(
        lines_for(&out, "ui/a.ts:"),
        ["ui/a.ts:1:8 boundary/folder-imports [advisory] db/x"]
    );
    assert!(out.ends_with("blackhole ci: red (protected)\n"), "{out}");
}

#[test]
fn non_null_wrapped_bases_and_specifiers_keep_their_findings() {
    let out = judge(
        &[
            (
                "blackhole.toml",
                "[[boundaries.folders]]\nfrom='ui/**'\ndeny=['restricted']",
            ),
            (
                "ui/a.ts",
                concat!(
                    "import {createRequire} from 'module';\n",
                    "createRequire((__filename)!)('allowed');\n",
                    "require((('restricted' satisfies string)!));\n",
                    "createRequire(import.meta.url)((`restricted`)!);",
                ),
            ),
        ],
        &[],
        1,
    );
    let folder: Vec<_> = lines_for(&out, "ui/a.ts:")
        .into_iter()
        .filter(|line| line.contains(" boundary/folder-imports "))
        .collect();
    assert_eq!(
        folder,
        [
            "ui/a.ts:2:30 boundary/folder-imports [advisory] specifier is not a literal",
            "ui/a.ts:3:11 boundary/folder-imports [advisory] restricted",
            "ui/a.ts:4:33 boundary/folder-imports [advisory] restricted",
        ]
    );
}
