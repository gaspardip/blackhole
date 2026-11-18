pub(super) mod bindings;
pub(super) mod expressions;
use oxc::ast::{
    AstKind,
    ast::{BindingPattern, TSType, TSTypeAnnotation, TSTypePredicateName},
};
pub(super) fn contains_unknown(ty: &TSType<'_>) -> bool {
    match ty.without_parenthesized() {
        TSType::TSUnknownKeyword(_) => true,
        TSType::TSUnionType(n) => n.types.iter().any(contains_unknown),
        _ => false,
    }
}

pub(super) fn binding_annotation(
    kind: AstKind<'_>,
) -> Option<(&BindingPattern<'_>, &TSTypeAnnotation<'_>)> {
    let (pattern, annotation) = match kind {
        AstKind::VariableDeclarator(v) => (&v.id, v.type_annotation.as_deref()?),
        AstKind::FormalParameter(p) => (&p.pattern, p.type_annotation.as_deref()?),
        AstKind::FormalParameterRest(p) => (&p.rest.argument, p.type_annotation.as_deref()?),
        _ => return None,
    };
    Some((pattern, annotation))
}

pub(super) fn parameter(kind: AstKind<'_>) -> Option<(Option<&str>, &TSType<'_>)> {
    let (name, a) = match kind {
        AstKind::TSThisParameter(p) => (Some("this"), p.type_annotation.as_deref()?),
        AstKind::VariableDeclarator(_) => return None,
        _ => {
            let (pattern, a) = binding_annotation(kind)?;
            (pattern.get_identifier_name().map(|i| i.as_str()), a)
        }
    };
    Some((name, a.type_annotation.without_parenthesized()))
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
