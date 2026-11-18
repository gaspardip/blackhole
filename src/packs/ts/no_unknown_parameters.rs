use crate::engine::{Family, Findings, Rule, Run, TsCtx};

pub(crate) const RULE: Rule = Rule {
    id: "ts/no-unknown-parameters",
    family: Family::Types,
    provenance: "Port of anti-slop no-unknown-parameters.",
    fix: "Accept a named domain type. Parse input at the I/O boundary before calling this function.",
    run: Run::Ts(check),
};

use super::function_types::{contains_unknown, parameter, returns};
use oxc::{
    ast::ast::{TSType, TSTypePredicateName},
    span::GetSpan,
};
fn check(ctx: &TsCtx<'_>, findings: &mut Findings) {
    let nodes = ctx.semantic.nodes();
    for (id, n) in nodes.iter_enumerated() {
        let Some((pattern, ty)) = parameter(n.kind()) else {
            continue;
        };
        let name = pattern.get_identifier_name();
        if !contains_unknown(ty) || name.is_some_and(|n| n == "cause") {
            continue;
        }
        let owner = nodes.parent_kind(nodes.parent_id(id));
        if returns(owner).is_some_and(|a|matches!(&a.type_annotation,TSType::TSTypePredicate(p) if matches!(&p.parameter_name,TSTypePredicateName::Identifier(i) if Some(i.name)==name))) {continue;}
        findings.report(ty.span().start as usize, None);
    }
}
