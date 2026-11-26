use super::module_object::{Kinds, Value};
use crate::{engine::TsCtx, packs::ts::values::unwrapped};
use oxc::ast::ast::{AssignmentTarget, AssignmentTargetProperty};
use oxc::{ast::AstKind, span::GetSpan, syntax::node::NodeId};

pub(super) fn declared_here<'a>(
    kinds: &Kinds<'_, 'a>,
    declarator: &oxc::ast::ast::VariableDeclarator<'a>,
    binding: &oxc::ast::ast::BindingPattern<'a>,
) -> bool {
    let Some(symbol) = binding
        .get_binding_identifier()
        .and_then(|id| id.symbol_id.get())
    else {
        return false;
    };
    matches!(
        kinds.values.declaration_for_symbol(symbol),
        Some(AstKind::VariableDeclarator(owner)) if owner.node_id.get() == declarator.node_id.get()
    )
}

pub(super) fn binding_is_read<'a>(
    kinds: &Kinds<'_, 'a>,
    v: &'a oxc::ast::ast::VariableDeclarator<'a>,
) -> bool {
    matches!(kinds.ctx.semantic.nodes().parent_kind(v.node_id.get()), AstKind::VariableDeclaration(d) if d.kind == oxc::ast::ast::VariableDeclarationKind::Const)
        && matches!(&v.id, oxc::ast::ast::BindingPattern::ObjectPattern(p) if kinds.values.simple_pattern(p) && kinds.ctx.counted(p.properties.iter()).all(|prop| declared_here(kinds,v,&prop.value)))
}

pub(super) fn base_write<'a>(kinds: &Kinds<'_, 'a>, id: NodeId, kind: AstKind<'a>) -> bool {
    let ctx = kinds.ctx;
    if matches!(
        kind,
        AstKind::StaticMemberExpression(_) | AstKind::ComputedMemberExpression(_)
    ) {
        if read_in_parent(outer(ctx, id), kind.span()) {
            return false;
        }
        let owner = match kind {
            AstKind::StaticMemberExpression(m) if m.property.name == "__filename" => {
                Some(&m.object)
            }
            AstKind::ComputedMemberExpression(m)
                if kinds.values.literal(&m.expression) == Some("__filename") =>
            {
                Some(&m.object)
            }
            _ => None,
        };
        return owner.and_then(|owner| kinds.kind_of(owner)) == Some(Value::Global);
    }
    match kind {
        AstKind::IdentifierReference(i) if i.name == "__filename" && kinds.runtime_global(i) => {
            i.reference_id
                .get()
                .is_some_and(|r| ctx.semantic.scoping().get_reference(r).flags().is_write())
                || matches!(outer(ctx,id), Some(AstKind::UnaryExpression(u)) if u.operator.is_delete())
        }
        AstKind::ImportMeta(_) => !meta_member_read(kinds, id),
        _ => false,
    }
}

pub(super) fn wrapped_node(kind: AstKind<'_>) -> Option<NodeId> {
    match kind {
        AstKind::SequenceExpression(s) => Some(s.expressions.last()?.node_id()),
        _ => crate::packs::ts::values::transparent_child(kind),
    }
}

pub(super) fn outer<'a>(ctx: &TsCtx<'a>, mut id: NodeId) -> Option<AstKind<'a>> {
    for parent in ctx.ancestors(id) {
        if wrapped_node(parent.kind()) != Some(id) {
            return Some(parent.kind());
        }
        id = parent.id();
    }
    None
}

fn meta_member_read(kinds: &Kinds<'_, '_>, id: NodeId) -> bool {
    let ctx = kinds.ctx;
    let nodes = ctx.semantic.nodes();
    let Some(member) = outer(ctx, id) else {
        return false;
    };
    let span = nodes.kind(id).span();
    let obj = match member {
        AstKind::StaticMemberExpression(m) => &m.object,
        AstKind::ComputedMemberExpression(m) if kinds.values.literal(&m.expression).is_some() => {
            &m.object
        }
        AstKind::VariableDeclarator(v) => {
            return matches!(&v.id, oxc::ast::ast::BindingPattern::ObjectPattern(p) if p.rest.is_none() && ctx.counted(p.properties.iter()).all(|p| kinds.values.key(&p.key).is_some()));
        }
        AstKind::AssignmentExpression(a) if a.right.span().contains_inclusive(span) => {
            return matches!(&a.left, AssignmentTarget::ObjectAssignmentTarget(p) if p.rest.is_none() && kinds.ctx.counted(p.properties.iter()).all(|prop| match prop {
                AssignmentTargetProperty::AssignmentTargetPropertyIdentifier(_) => true,
                AssignmentTargetProperty::AssignmentTargetPropertyProperty(p) => kinds.values.key(&p.name).is_some(),
            }));
        }
        _ => return false,
    };
    unwrapped(ctx, obj).span() == span
        && read_in_parent(outer(ctx, member.node_id()), member.span())
}

pub(super) fn read_in_parent(parent: Option<AstKind<'_>>, span: oxc::span::Span) -> bool {
    match parent {
        Some(AstKind::AssignmentExpression(a)) => !a.left.span().contains_inclusive(span),
        Some(AstKind::ForOfStatement(f)) => !f.left.span().contains_inclusive(span),
        Some(AstKind::ForInStatement(f)) => !f.left.span().contains_inclusive(span),
        Some(AstKind::UnaryExpression(u)) => !u.operator.is_delete(),
        Some(AstKind::TaggedTemplateExpression(t)) => !t.tag.span().contains_inclusive(span),
        Some(
            AstKind::UpdateExpression(_)
            | AstKind::ArrayAssignmentTarget(_)
            | AstKind::AssignmentTargetRest(_),
        ) => false,
        Some(AstKind::AssignmentTargetPropertyProperty(p)) => {
            !p.binding.span().contains_inclusive(span)
        }
        Some(AstKind::AssignmentTargetWithDefault(a)) => !a.binding.span().contains_inclusive(span),
        _ => true,
    }
}
