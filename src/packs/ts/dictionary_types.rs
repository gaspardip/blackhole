use crate::engine::TsCtx;
use oxc::{
    ast::{
        AstKind,
        ast::{TSSignature, TSType, TSTypeName, TSTypeOperatorOperator},
    },
    syntax::{node::NodeId, symbol::SymbolId},
};
use std::collections::{HashMap, HashSet};

#[derive(Clone, Copy, PartialEq)]
pub(super) enum Class {
    None,
    Broad,
    Dictionary,
    Any,
}
#[derive(Clone, Copy, PartialEq)]
enum Query {
    UnknownReturn,
    Target,
    AliasTarget,
    Key,
    Value,
}
#[derive(Clone, Default)]
struct Resolution<'a> {
    arguments: HashMap<SymbolId, &'a TSType<'a>>,
    visiting: HashSet<NodeId>,
}
pub(super) fn target<'a>(ctx: &TsCtx<'a>, ty: &'a TSType<'a>) -> Class {
    Resolution::default().classify(ctx, ty, Query::Target)
}
pub(super) fn informative<'a>(ctx: &TsCtx<'a>, ty: &'a TSType<'a>) -> bool {
    Resolution::default().classify(ctx, ty, Query::Value) == Class::None
}

pub(super) fn unknown_return<'a>(ctx: &TsCtx<'a>, ty: &'a TSType<'a>) -> bool {
    Resolution::default().classify(ctx, ty, Query::UnknownReturn) != Class::None
}

fn empty(members: &[TSSignature<'_>]) -> bool {
    members.iter().all(|m|matches!(m,TSSignature::TSPropertySignature(p) if p.optional && p.type_annotation.as_ref().is_some_and(|a|matches!(transparent(&a.type_annotation),TSType::TSNeverKeyword(_)))))
}

fn transparent<'a>(mut ty: &'a TSType<'a>) -> &'a TSType<'a> {
    loop {
        ty = match ty {
            TSType::TSParenthesizedType(n) => &n.type_annotation,
            TSType::TSTypeOperatorType(n) if n.operator == TSTypeOperatorOperator::Readonly => {
                &n.type_annotation
            }
            _ => return ty,
        };
    }
}

impl<'a> Resolution<'a> {
    fn reference(
        &self,
        ctx: &TsCtx<'a>,
        r: &'a oxc::ast::ast::TSTypeReference<'a>,
        query: Query,
    ) -> Class {
        let TSTypeName::IdentifierReference(name) = &r.type_name else {
            return Class::None;
        };
        let args = r
            .type_arguments
            .as_ref()
            .map_or(&[][..], |p| p.params.as_slice());
        let scopes = ctx.semantic.scoping();
        if let Some(symbol) = name
            .reference_id
            .get()
            .and_then(|id| scopes.get_reference(id).symbol_id())
        {
            let id = scopes.symbol_declaration(symbol);
            let mut next = self.clone();
            if !next.visiting.insert(id) {
                return Class::None;
            }
            if let Some(arg) = self.arguments.get(&symbol) {
                return next.classify(ctx, arg, query);
            }
            return match ctx.semantic.nodes().kind(id) {
                AstKind::TSInterfaceDeclaration(i)
                    if query == Query::Value
                        && scopes.symbol_declarations(symbol).count() == 1
                        && i.extends.is_empty()
                        && empty(&i.body.body) =>
                {
                    Class::Broad
                }
                AstKind::TSTypeAliasDeclaration(a) => {
                    if let Some(params) = &a.type_parameters {
                        for (i, p) in params.params.iter().enumerate() {
                            let Some(arg) = args.get(i).or(p.default.as_ref()) else {
                                return Class::None;
                            };
                            next.arguments.insert(p.name.symbol_id.get().unwrap(), arg);
                        }
                    }
                    let inner = next.classify(
                        ctx,
                        &a.type_annotation,
                        if query == Query::Target {
                            Query::AliasTarget
                        } else {
                            query
                        },
                    );
                    if query == Query::Target
                        && a.type_parameters.is_some()
                        && inner != Class::Dictionary
                    {
                        Class::None
                    } else {
                        inner
                    }
                }
                _ => Class::None,
            };
        }
        match name.name.as_str() {
            "Readonly" | "NonNullable" | "Partial" | "Required"
                if matches!(query, Query::Target | Query::AliasTarget | Query::Value) =>
            {
                args.first()
                    .map_or(Class::None, |v| self.classify(ctx, v, query))
            }
            "Promise" | "PromiseLike" if query == Query::UnknownReturn => args
                .first()
                .map_or(Class::None, |v| self.classify(ctx, v, query)),
            "Record"
                if matches!(query, Query::Target | Query::AliasTarget)
                    && args
                        .first()
                        .is_none_or(|v| self.classify(ctx, v, Query::Key) != Class::None) =>
            {
                Class::Dictionary
            }
            "PropertyKey" if query == Query::Key => Class::Broad,
            _ => Class::None,
        }
    }
    fn classify(&self, ctx: &TsCtx<'a>, ty: &'a TSType<'a>, query: Query) -> Class {
        let ty = transparent(ty);
        if let TSType::TSTypeReference(r) = ty {
            return self.reference(ctx, r, query);
        }
        match (ty, query) {
            (TSType::TSUnknownKeyword(_), Query::UnknownReturn)
            | (
                TSType::TSUnknownKeyword(_) | TSType::TSObjectKeyword(_),
                Query::Value | Query::Target | Query::AliasTarget,
            )
            | (
                TSType::TSStringKeyword(_)
                | TSType::TSNumberKeyword(_)
                | TSType::TSSymbolKeyword(_),
                Query::Key,
            ) => Class::Broad,
            (TSType::TSAnyKeyword(_), Query::Value) => Class::Any,
            (TSType::TSUnionType(n), Query::Value | Query::Key | Query::UnknownReturn)
                if n.types
                    .iter()
                    .any(|m| self.classify(ctx, m, query) != Class::None) =>
            {
                Class::Broad
            }
            (TSType::TSIntersectionType(n), Query::Value) => {
                let mut all = true;
                for m in &n.types {
                    match self.classify(ctx, m, query) {
                        Class::Any => return Class::Any,
                        Class::None => all = false,
                        _ => {}
                    }
                }
                if all && !n.types.is_empty() {
                    Class::Broad
                } else {
                    Class::None
                }
            }
            (TSType::TSTypeLiteral(n), Query::Value) if empty(&n.members) => Class::Broad,
            (TSType::TSTypeLiteral(n), Query::Target | Query::AliasTarget)
                if n.members
                    .iter()
                    .any(|m| matches!(m, TSSignature::TSIndexSignature(_))) =>
            {
                Class::Dictionary
            }
            (TSType::TSTypeLiteral(n), Query::Target) if !n.members.is_empty() => Class::Broad,
            (TSType::TSMappedType(_), Query::Target) => Class::Dictionary,
            (TSType::TSMappedType(n), Query::AliasTarget)
                if self.classify(ctx, &n.constraint, Query::Key) != Class::None =>
            {
                Class::Dictionary
            }
            _ => Class::None,
        }
    }
}
