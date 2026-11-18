pub(super) mod bindings;
pub(super) mod expressions;
use oxc::ast::{
    AstKind,
    ast::{TSType, TSTypeAnnotation, TSTypePredicateName},
};
pub(super) fn contains_unknown(ty: &TSType<'_>) -> bool {
    let mut pending = vec![ty];
    while let Some(ty) = pending.pop() {
        match unparen(ty) {
            TSType::TSUnknownKeyword(_) => return true,
            TSType::TSUnionType(n) => pending.extend(&n.types),
            _ => {}
        }
    }
    false
}

pub(super) fn unparen<'a>(mut ty: &'a TSType<'a>) -> &'a TSType<'a> {
    while let TSType::TSParenthesizedType(p) = ty {
        ty = &p.type_annotation;
    }
    ty
}

pub(super) fn parameter(kind: AstKind<'_>) -> Option<(Option<&str>, &TSType<'_>)> {
    let (name, a) = match kind {
        AstKind::FormalParameter(p) => (
            p.pattern.get_identifier_name().map(|i| i.as_str()),
            p.type_annotation.as_ref()?,
        ),
        AstKind::FormalParameterRest(p) => (
            p.rest.argument.get_identifier_name().map(|i| i.as_str()),
            p.type_annotation.as_ref()?,
        ),
        AstKind::TSThisParameter(p) => (Some("this"), p.type_annotation.as_ref()?),
        _ => return None,
    };
    Some((name, unparen(&a.type_annotation)))
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

pub(super) fn predicate_subject<'a>(annotation: &TSTypeAnnotation<'a>) -> Option<&'a str> {
    let TSType::TSTypePredicate(predicate) = &annotation.type_annotation else {
        return None;
    };
    let TSTypePredicateName::Identifier(name) = &predicate.parameter_name else {
        return None;
    };
    Some(name.name.as_str())
}
