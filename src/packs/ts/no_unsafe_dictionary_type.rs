use crate::engine::{Family, Findings, Rule, Run, TsCtx};

pub(crate) const RULE: Rule = Rule {
    id: "ts/no-unsafe-dictionary-type",
    family: Family::Types,
    provenance: "Port of anti-slop no-unsafe-dictionary-type.",
    fix: "Use an owner or schema-derived value type. Parse external payloads before inserting them.",
    run: Run::Ts(check),
};

use super::{
    dictionary_types::{informative, unsafe_dictionary},
    function_types::bindings::declaration,
};
use oxc::{
    ast::{AstKind, ast::TSTypeName},
    span::GetSpan,
};

fn check(ctx: &TsCtx<'_>, findings: &mut Findings) {
    let nodes = ctx.semantic.nodes();
    for (id, node) in nodes.iter_enumerated() {
        let kind = node.kind();
        if let AstKind::TSIndexSignature(n) = kind {
            if !matches!(nodes.parent_kind(id), AstKind::TSTypeLiteral(_))
                && !informative(ctx, &n.type_annotation.type_annotation)
            {
                findings.report(n.span.start as usize, None);
            }
            continue;
        }
        if !unsafe_dictionary(ctx, kind) {
            continue;
        }
        let ancestors = || nodes.ancestor_ids(id).map(|a| nodes.kind(a));
        if let AstKind::TSTypeReference(r) = kind
            && r.type_arguments.is_none()
            && let TSTypeName::IdentifierReference(i) = &r.type_name
            && declaration(ctx, i)
                .is_some_and(|d| matches!(nodes.kind(d), AstKind::TSTypeAliasDeclaration(_)))
            && !ancestors().any(|k| matches!(k, AstKind::TSTypeAliasDeclaration(_)))
        {
            continue;
        }
        if ancestors().any(|k|matches!(k,AstKind::TSTypeParameter(p) if p.constraint.as_ref().is_some_and(|t|t.span().contains_inclusive(node.span()))) || unsafe_dictionary(ctx,k)){continue;}
        findings.report(node.span().start as usize, None);
    }
}
