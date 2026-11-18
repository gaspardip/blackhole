pub(super) mod bindings;
pub(super) mod expressions;
use oxc::ast::{
    AstKind,
    ast::{BindingPattern, TSType, TSTypeAnnotation},
};
pub(super) fn contains_unknown(ty: &TSType<'_>) -> bool {
    match ty.without_parenthesized() {
        TSType::TSUnknownKeyword(_) => true,
        TSType::TSUnionType(n) => n.types.iter().any(contains_unknown),
        _ => false,
    }
}
pub(super) fn parameter(kind: AstKind<'_>) -> Option<(&BindingPattern<'_>, &TSType<'_>)> {
    let (p, a) = match kind {
        AstKind::FormalParameter(p) => (&p.pattern, p.type_annotation.as_ref()?),
        AstKind::FormalParameterRest(p) => (&p.rest.argument, p.type_annotation.as_ref()?),
        _ => return None,
    };
    Some((p, &a.type_annotation))
}
pub(super) fn returns(kind: AstKind<'_>) -> Option<&TSTypeAnnotation<'_>> {
    match kind {
        AstKind::Function(n) => n.return_type.as_deref(),
        AstKind::ArrowFunctionExpression(n) => n.return_type.as_deref(),
        AstKind::TSCallSignatureDeclaration(n) => n.return_type.as_deref(),
        AstKind::TSConstructSignatureDeclaration(n) => n.return_type.as_deref(),
        AstKind::TSConstructorType(n) => Some(&n.return_type),
        AstKind::TSFunctionType(n) => Some(&n.return_type),
        AstKind::TSMethodSignature(n) => n.return_type.as_deref(),
        _ => None,
    }
}
