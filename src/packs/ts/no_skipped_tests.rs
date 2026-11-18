use super::test_context::{chain_property, is_test, root, suite, test_file};
use crate::engine::{Family, Findings, Rule, Run, TsCtx};
use oxc::{
    ast::{AstKind, ast::Expression},
    span::GetSpan,
};
use std::collections::BTreeSet;
pub(crate) const RULE: Rule = Rule {
    id: "ts/no-skipped-tests",
    family: Family::Tests,
    provenance: "Agents skip, focus or stub out a failing test to get a green run.",
    fix: "Make the test pass or delete it with the behaviour it covered. A skipped test proves nothing.",
    run: Run::Ts(check),
};
fn check(ctx: &TsCtx<'_>, findings: &mut Findings) {
    let mut locations = BTreeSet::new();
    let nodes = ctx.semantic.nodes();
    for (id, n) in nodes.iter_enumerated() {
        let AstKind::CallExpression(c) = n.kind() else {
            continue;
        };
        let base = root(&c.callee);
        let position=if base.is_some_and(|s|is_test(s)||suite(s)){
            chain_property(&c.callee,|s|matches!(s,"skip"|"only"|"todo"|"skipIf"|"runIf"|"fixme"|"fails"|"failing"))
        }else if matches!(&c.callee,Expression::Identifier(i) if matches!(i.name.as_str(),"xit"|"xtest"|"xdescribe"|"fit"|"fdescribe")){
            Some(c.callee.span().start as usize)
        }else if test_file(ctx.path) && nodes.ancestors(id).any(|n|matches!(n.kind(),AstKind::Function(_)|AstKind::ArrowFunctionExpression(_)) && matches!(nodes.parent_kind(n.id()),AstKind::CallExpression(parent) if root(&parent.callee).is_some_and(is_test))){
            match &c.callee{Expression::StaticMemberExpression(m) if matches!(&m.object,Expression::ThisExpression(_)) && m.property.name=="skip"=>Some(m.property.span.start as usize),Expression::Identifier(i) if i.name=="pending"=>Some(i.span.start as usize),_=>None}
        }else{None};
        locations.extend(position);
    }
    for at in locations {
        findings.report(at, None);
    }
}
