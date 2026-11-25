use super::values::{contains_shape, static_string_contains_shape};
use crate::engine::{Family, Findings, Rule, Run, TsCtx};
use oxc::{ast::AstKind, span::GetSpan};
use std::collections::HashSet;

pub(crate) const RULE: Rule = Rule {
    id: "ts/no-shape-in-symbol-names",
    family: Family::Slop,
    provenance: "Port of anti-slop no-shape-in-symbol-names.",
    fix: "Rename the symbol for the domain value it represents.",
    run: Run::Ts(check),
};

fn type_path(kind: &AstKind<'_>) -> bool {
    matches!(
        kind,
        AstKind::TSQualifiedName(_)
            | AstKind::TSImportType(_)
            | AstKind::TSImportTypeQualifiedName(_)
    )
}

fn check(ctx: &TsCtx<'_>, findings: &mut Findings) {
    let nodes = ctx.semantic.nodes();
    let mut reported = HashSet::new();
    let mut queried = HashSet::new();
    for (id, node) in nodes.iter_enumerated() {
        let kind = node.kind();
        // Parents come before children, so one lookup marks a path under a `typeof` query.
        if type_path(&kind)
            && (matches!(nodes.parent_kind(id), AstKind::TSTypeQuery(_))
                || queried.contains(&nodes.parent_id(id)))
        {
            queried.insert(id);
        }
        let property = match kind {
            AstKind::ObjectProperty(p) => Some(&p.key),
            AstKind::TSPropertySignature(p) => Some(&p.key),
            AstKind::TSMethodSignature(p) => Some(&p.key),
            AstKind::PropertyDefinition(p) => Some(&p.key),
            AstKind::MethodDefinition(p) => Some(&p.key),
            AstKind::AccessorProperty(p) => Some(&p.key),
            _ => None,
        };
        let (has_shape, span) = if let Some(property) = property {
            let expression = property.as_expression();
            (
                expression.is_some_and(|expr| static_string_contains_shape(ctx, expr)),
                expression.map_or_else(|| property.span(), |e| e.get_inner_expression().span()),
            )
        } else {
            let name = match kind {
                AstKind::IdentifierName(n) => match nodes.parent_kind(id) {
                    AstKind::StaticMemberExpression(m) if m.property.span == n.span => continue,
                    AstKind::BindingProperty(p) if p.key.span() == n.span => continue,
                    AstKind::AssignmentTargetPropertyProperty(p) if p.name.span() == n.span => {
                        continue;
                    }
                    // Names after the root of a `typeof` query belong to another module or object.
                    _ if queried.contains(&nodes.parent_id(id)) => continue,
                    _ => n.name.as_bytes(),
                },
                AstKind::IdentifierReference(n) => n.name.as_bytes(),
                AstKind::BindingIdentifier(n) => n.name.as_bytes(),
                AstKind::PrivateIdentifier(n)
                    if !matches!(nodes.parent_kind(id), AstKind::PrivateFieldExpression(_)) =>
                {
                    n.name.as_bytes()
                }
                AstKind::JSXIdentifier(n) => match nodes.parent_kind(id) {
                    AstKind::JSXMemberExpression(m) if m.property.span == n.span => continue,
                    AstKind::JSXAttribute(a) if a.name.span() == n.span => continue,
                    AstKind::JSXNamespacedName(_) => continue,
                    _ => n.name.as_bytes(),
                },
                AstKind::LabelIdentifier(n) => n.name.as_bytes(),
                AstKind::StringLiteral(literal) => {
                    let named = match nodes.parent_kind(id) {
                        AstKind::ImportSpecifier(_) | AstKind::ExportSpecifier(_) => true,
                        AstKind::ImportAttribute(a) => a.key.span() == literal.span,
                        AstKind::TSEnumMember(m) => m.id.span() == literal.span,
                        AstKind::ExportAllDeclaration(e) => e
                            .exported
                            .as_ref()
                            .is_some_and(|n| n.span() == literal.span),
                        _ => false,
                    };
                    if !named {
                        continue;
                    }
                    literal.value.as_bytes()
                }
                AstKind::TemplateLiteral(t) if matches!(nodes.parent_kind(id),AstKind::TSEnumMember(m) if m.id.span()==t.span) =>
                {
                    let Some(name) = t.single_quasi() else {
                        continue;
                    };
                    name.as_bytes()
                }
                _ => continue,
            };
            (contains_shape(name), node.span())
        };
        if has_shape && reported.insert(span) {
            findings.report(span.start as usize, None);
        }
    }
}
