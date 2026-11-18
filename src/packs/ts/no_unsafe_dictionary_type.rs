use super::{
    dictionary_types::{Resolution, Unjudged, report},
    function_types::bindings::symbol,
};
use crate::engine::{Family, Findings, Rule, Run, TsCtx};
use oxc::{
    ast::{AstKind, ast::TSTypeName},
    span::GetSpan,
};

pub(crate) const RULE: Rule = Rule {
    id: "ts/no-unsafe-dictionary-type",
    family: Family::Types,
    provenance: "Port of anti-slop no-unsafe-dictionary-type.",
    fix: "Use an owner or schema-derived value type. Parse external payloads before inserting them.",
    run: Run::Ts(check),
};

fn is_bare_alias_use(ctx: &TsCtx<'_>, kind: AstKind<'_>) -> bool {
    let AstKind::TSTypeReference(r) = kind else {
        return false;
    };
    let TSTypeName::IdentifierReference(ident) = &r.type_name else {
        return false;
    };
    r.type_arguments.is_none()
        && symbol(ctx, ident)
            .is_some_and(|s| ctx.semantic.scoping().symbol_flags(s).is_type_alias())
}

fn inside_constraint(parent: AstKind<'_>, span: oxc::span::Span) -> bool {
    let AstKind::TSTypeParameter(p) = parent else {
        return false;
    };
    p.constraint
        .as_ref()
        .is_some_and(|t| t.span().contains_inclusive(span))
}

fn check(ctx: &TsCtx<'_>, findings: &mut Findings) {
    check_with_resolution(ctx, findings, &Resolution::default());
}

pub(super) fn check_with_resolution(
    ctx: &TsCtx<'_>,
    findings: &mut Findings,
    resolution: &Resolution,
) {
    let nodes = ctx.semantic.nodes();
    let mut dictionaries = std::collections::HashSet::new();
    for (id, node) in nodes.iter_enumerated() {
        resolution.begin_candidate();
        let kind = node.kind();
        if let AstKind::TSIndexSignature(n) = kind {
            if !matches!(nodes.parent_kind(id), AstKind::TSTypeLiteral(_)) {
                report(
                    findings,
                    n.span.start as usize,
                    resolution
                        .informative(ctx, &n.type_annotation.type_annotation)
                        .map(|v| !v),
                );
            }
            continue;
        }
        match resolution.unsafe_dictionary(ctx, kind) {
            Ok(true) => {}
            Ok(false) => continue,
            Err(Unjudged) => {
                report(findings, node.span().start as usize, Err(Unjudged));
                continue;
            }
        }
        dictionaries.insert(id);
        let mut needs_alias = is_bare_alias_use(ctx, kind);
        let hidden = nodes.ancestor_ids(id).any(|id| {
            let parent = nodes.kind(id);
            if matches!(parent, AstKind::TSTypeAliasDeclaration(_)) {
                needs_alias = false;
            }
            dictionaries.contains(&id) || inside_constraint(parent, node.span())
        });
        if !hidden && !needs_alias {
            findings.report(node.span().start as usize, None);
        }
    }
}
