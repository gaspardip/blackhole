use blackhole_check::engine::{registry, run};
use std::fmt::Write;

const RULE: &str = "ts/no-conditional-empty-object-spread";
const TS: &str = "case.ts";
const MARKED_SPREAD: &str = "consume({ ^...(c ? empty : x) });";

#[derive(Default)]
struct Cases {
    reports: usize,
    silent: usize,
    disagreements: Vec<String>,
}

impl Cases {
    fn check(&mut self, path: &str, label: &str, marked: &str) {
        let expected: Vec<_> = marked
            .split_once('^')
            .map(|(prefix, suffix)| {
                assert!(suffix.starts_with("..."), "{label}: {marked}");
                (
                    RULE,
                    prefix.split('\n').count(),
                    prefix.rsplit('\n').next().unwrap().len() + 1,
                )
            })
            .into_iter()
            .collect();
        let source = marked.replacen('^', "", 1);
        if expected.is_empty() {
            self.silent += 1;
        } else {
            self.reports += 1;
        }
        let actual: Vec<_> = run(registry(), [(path, source.as_bytes())])
            .into_iter()
            .filter(|hit| matches!(hit.rule, RULE | "core/unparseable"))
            .map(|hit| (hit.rule, hit.line, hit.col))
            .collect();
        if actual != expected {
            self.disagreements.push(format!(
                "{label} ({path}): expected {expected:?}, got {actual:?}\n{source}"
            ));
        }
    }

    fn pair(&mut self, path: &str, label: &str, control: &str, silent: &str) {
        self.check(path, &format!("{label} control"), control);
        self.check(path, label, &silent.replace('^', ""));
    }

    fn finish(self) {
        eprintln!("{} reporting, {} silent", self.reports, self.silent);
        assert!(
            self.disagreements.is_empty(),
            "{} disagreements:\n{}",
            self.disagreements.len(),
            self.disagreements.join("\n\n")
        );
    }
}

const WRAPPERS: &[&str] = &[
    "(VALUE)",
    "(VALUE as T)",
    "VALUE!",
    "(VALUE satisfies T)",
    "<T>VALUE",
];

fn wrap(template: &str, value: &str) -> String {
    template.replace("VALUE", value)
}

fn conditionals(depth: usize) -> Vec<String> {
    let mut arms = vec![String::from("VALUE")];
    for _ in 0..=depth {
        arms = arms
            .iter()
            .flat_map(|arm| [format!("(c ? {arm} : x)"), format!("(c ? x : {arm})")])
            .collect();
    }
    arms
}

fn report(cases: &mut Cases, path: &str, branch: &str, ternary: &str, operand: &str) {
    let spread = wrap(operand, &wrap(ternary, branch));
    cases.check(
        path,
        "conditional",
        &format!("consume({{ ^...{spread} }});"),
    );
}

#[test]
fn literal_and_wrapped_conditionals() {
    let mut cases = Cases::default();
    for (path, wrappers) in [(TS, WRAPPERS), ("case.tsx", &WRAPPERS[..4])] {
        for depth in 0..=2 {
            for ternary in conditionals(depth) {
                report(&mut cases, path, "{}", &ternary, "VALUE");
                for &outer in wrappers {
                    let branch = wrap(outer, "{}");
                    report(&mut cases, path, &branch, &ternary, "VALUE");
                    report(&mut cases, path, "{}", &ternary, outer);
                    for &inner in wrappers {
                        report(&mut cases, path, &branch, &ternary, inner);
                        if outer != inner {
                            let combined = wrap(outer, inner);
                            report(&mut cases, path, &wrap(&combined, "{}"), &ternary, "VALUE");
                            report(&mut cases, path, "{}", &ternary, &combined);
                        }
                    }
                }
            }
        }
    }
    cases.finish();
}

const BINDINGS: [&str; 3] = ["empty", "alias", "again"];
const INITIALIZERS: [&str; 3] = ["{}", "empty", "alias"];

fn declarations(kinds: &[&str], annotations: usize) -> String {
    let mut source = String::new();
    for (index, (kind, (name, initializer))) in kinds
        .iter()
        .zip(BINDINGS.into_iter().zip(INITIALIZERS))
        .enumerate()
    {
        let annotation = if annotations & (1 << index) == 0 {
            ""
        } else {
            ": object"
        };
        writeln!(source, "{kind} {name}{annotation} = {initializer};").unwrap();
    }
    source
}

#[test]
fn const_aliases_and_harmless_uses() {
    let mut cases = Cases::default();
    for (index, binding) in BINDINGS.into_iter().enumerate() {
        let length = index + 1;
        let spread = MARKED_SPREAD.replace("empty", binding);
        for annotations in 0..1 << length {
            let declarations = declarations(&["const"; 3][..length], annotations);
            for (label, extra) in [
                ("only the spread", String::new()),
                ("discarded read", format!("void {binding};")),
                ("unused alias", format!("const unused = {binding};")),
                (
                    "type-only re-export",
                    format!("export type {{ {binding} }} from './other';"),
                ),
            ] {
                cases.check(TS, label, &format!("{declarations}{extra}\n{spread}"));
            }
        }
    }
    cases.finish();
}

#[test]
fn conditionals_need_an_empty_arm_and_an_object_spread() {
    let mut cases = Cases::default();
    for ternary in ["(c ? {} : x)", "((c ? x : (d ? {} : x)) as T)"] {
        let control = format!("consume({{ ^...{ternary} }});");
        cases.pair(
            TS,
            "nonempty arm",
            &control,
            &control.replace("{}", "{ k: 1 }"),
        );
        cases.pair(
            TS,
            "array spread",
            &control,
            &format!("consume([...{ternary}]);"),
        );
        cases.pair(
            TS,
            "argument without a spread",
            &control,
            &format!("consume({ternary});"),
        );
    }
    cases.finish();
}

#[test]
fn observed_aliases_are_silent() {
    let mut cases = Cases::default();
    let declarations = declarations(&["const"; 3], 0);
    let spread = MARKED_SPREAD.replace("empty", "again");
    for (label, control, observation) in [
        ("escaped argument", "void again;", "consume(again);"),
        ("exported alias", "void again;", "export default again;"),
        ("type query", "void again;", "type Observed = typeof again;"),
        ("comma read", "void again;", "(again, log());"),
        (
            "interface sharing a name",
            "interface Other {}",
            "interface empty {}",
        ),
        (
            "type alias sharing a name",
            "type Other = {};",
            "type empty = {};",
        ),
    ] {
        cases.pair(
            TS,
            label,
            &format!("{declarations}{control}\n{spread}"),
            &format!("{declarations}{observation}\n{spread}"),
        );
    }

    let head = "const empty = {};\nconst left = c ? empty : right;\n";
    cases.pair(
        TS,
        "alias cycle",
        &format!(
            "const empty = {{}};\nconst left = c ? empty : x;\nconst right = left;\n{MARKED_SPREAD}"
        ),
        &format!("{head}const right = left;\n{MARKED_SPREAD}"),
    );
    cases.finish();
}

#[test]
fn reads_must_follow_declarations() {
    let mut cases = Cases::default();
    for (before, declaration, binding) in [
        ("", "const empty = {};", "empty"),
        ("const empty = {};\n", "const alias = empty;", "alias"),
    ] {
        for (label, prefix, suffix, invoke) in [
            ("direct read", "", "", ""),
            (
                "deferred function read",
                "function read() { ",
                " }",
                "read();",
            ),
            (
                "deferred class read",
                "class Holder { field = ",
                " }",
                "new Holder();",
            ),
        ] {
            let expression = format!("{{ ^...(c ? {binding} : x) }}");
            let read = format!("{prefix}({expression});{suffix}");
            cases.pair(
                TS,
                label,
                &format!("{before}{declaration}\n{read}\n{invoke}"),
                &format!("{before}{read}\n{declaration}\n{invoke}"),
            );
        }
    }
    cases.finish();
}

#[test]
fn every_binding_in_an_alias_chain_must_be_const() {
    let mut cases = Cases::default();
    let consts = ["const"; 3];
    let spread = MARKED_SPREAD.replace("empty", "again");
    let source = |kinds: &[&str]| {
        format!(
            "async function build() {{\n{}{spread}\n}}",
            declarations(kinds, 0)
        )
    };
    let control = source(&consts);
    for (index, name) in BINDINGS.into_iter().enumerate() {
        for kind in ["let", "var", "using", "await using"] {
            let mut kinds = consts;
            kinds[index] = kind;
            cases.pair(TS, &format!("{kind} {name}"), &control, &source(&kinds));
        }
    }
    cases.finish();
}

#[test]
fn member_reads_are_silent() {
    let mut cases = Cases::default();
    let prefix = "const empty = {};\nconst p = { empty: {} };\n";
    let member = MARKED_SPREAD.replace("empty", "p.empty");
    cases.pair(
        TS,
        "member projection",
        &format!("{prefix}{MARKED_SPREAD}"),
        &format!("{prefix}{member}"),
    );

    // Untouched member reads are already silent, so object uses have no reporting member control.
    for (label, observation) in [
        ("write", "p.empty = x;"),
        ("delete", "delete p.empty;"),
        ("argument", "consume(p);"),
        ("alias", "const alias = p;"),
        ("method", "p.fill();"),
        ("type query", "type Owner = typeof p;"),
    ] {
        cases.check(
            TS,
            label,
            &format!("{prefix}{observation}\n{}", member.replace('^', "")),
        );
    }
    cases.finish();
}

#[test]
fn dynamic_scope_blocks_aliases_but_keeps_literals() {
    let mut cases = Cases::default();
    let path = "case.cjs";
    let declaration = "const empty = {};\n";
    cases.pair(
        path,
        "with body",
        &format!("{declaration}if (scope) {{ {MARKED_SPREAD} }}"),
        &format!("{declaration}with (scope) {{ {MARKED_SPREAD} }}"),
    );
    cases.check(
        path,
        "literal in a with body",
        &format!(
            "with (scope) {{ {} }}",
            MARKED_SPREAD.replace("empty", "{}")
        ),
    );
    for (path, mention) in [
        (path, "eval(source);"),
        (path, "void eval;"),
        (path, "const eval = custom;"),
        (path, "const props = { eval: custom };"),
        (path, "void 'eval';"),
        (path, "void `eval`;"),
        (TS, "type Lookup = eval;"),
        ("case.tsx", "const view = <View eval={true} />;"),
    ] {
        let control = mention.replace("eval", "other");
        cases.pair(
            path,
            mention,
            &format!("{declaration}{control}\n{MARKED_SPREAD}"),
            &format!("{declaration}{mention}\n{MARKED_SPREAD}"),
        );
        cases.check(
            path,
            "literal in a file mentioning eval",
            &format!("{mention}\n{}", MARKED_SPREAD.replace("empty", "{}")),
        );
    }
    cases.finish();
}
