use crate::engine::TsCtx;
use oxc::{
    ast::{
        AstKind,
        ast::{TSSignature, TSType, TSTypeAliasDeclaration, TSTypeName, TSTypeOperatorOperator},
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
    UnsafeDictionary,
    ObjectInput,
    UnknownAlias,
    UnknownReturn,
    Target,
    AliasTarget,
    Key,
    Value,
}
#[derive(Default)]
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

pub(super) fn object_input<'a>(ctx: &TsCtx<'a>, ty: &'a TSType<'a>) -> bool {
    Resolution::default().classify(ctx, ty, Query::ObjectInput) != Class::None
}
pub(super) fn unknown_alias<'a>(ctx: &TsCtx<'a>, ty: &'a TSType<'a>) -> bool {
    Resolution::default().classify(ctx, ty, Query::UnknownAlias) != Class::None
}
pub(super) fn unknown_return<'a>(ctx: &TsCtx<'a>, ty: &'a TSType<'a>) -> bool {
    Resolution::default().classify(ctx, ty, Query::UnknownReturn) != Class::None
}

fn empty(members: &[TSSignature<'_>]) -> bool {
    members.iter().all(optional_never_property)
}

fn optional_never_property(member: &TSSignature<'_>) -> bool {
    let TSSignature::TSPropertySignature(property) = member else {
        return false;
    };
    property.optional
        && property
            .type_annotation
            .as_ref()
            .is_some_and(|a| matches!(transparent(&a.type_annotation), TSType::TSNeverKeyword(_)))
}

fn transparent<'a>(mut ty: &'a TSType<'a>) -> &'a TSType<'a> {
    loop {
        ty = ty.without_parenthesized();
        ty = match ty {
            TSType::TSTypeOperatorType(n) if n.operator == TSTypeOperatorOperator::Readonly => {
                &n.type_annotation
            }
            _ => return ty,
        };
    }
}

impl<'a> Resolution<'a> {
    fn index_values(&mut self, ctx: &TsCtx<'a>, members: &'a [TSSignature<'a>]) -> Class {
        members
            .iter()
            .find_map(|m| {
                let TSSignature::TSIndexSignature(i) = m else {
                    return None;
                };
                let value = self.classify(ctx, &i.type_annotation.type_annotation, Query::Value);
                (value != Class::None).then_some(value)
            })
            .unwrap_or(Class::None)
    }
    fn alias(
        &mut self,
        ctx: &TsCtx<'a>,
        alias: &'a TSTypeAliasDeclaration<'a>,
        args: &'a [TSType<'a>],
        query: Query,
    ) -> Class {
        let params = alias
            .type_parameters
            .as_ref()
            .map_or(&[][..], |p| p.params.as_slice());
        if params.iter().skip(args.len()).any(|p| p.default.is_none()) {
            return Class::None;
        }
        // Each active alias owns distinct parameter symbols, and visiting prevents alias re-entry.
        for (i, p) in params.iter().enumerate() {
            let arg = args.get(i).or(p.default.as_ref()).unwrap();
            self.arguments.insert(p.name.symbol_id.get().unwrap(), arg);
        }
        let inner = self.classify(
            ctx,
            &alias.type_annotation,
            if query == Query::Target {
                Query::AliasTarget
            } else {
                query
            },
        );
        for p in params {
            self.arguments.remove(&p.name.symbol_id.get().unwrap());
        }
        if query == Query::Target && alias.type_parameters.is_some() && inner != Class::Dictionary {
            Class::None
        } else {
            inner
        }
    }
    fn reference(
        &mut self,
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
        if let Some(symbol) = super::function_types::bindings::symbol(ctx, name) {
            let id = scopes.symbol_declaration(symbol);
            if !self.visiting.insert(id) {
                return Class::None;
            }
            let result = if let Some(&arg) = self.arguments.get(&symbol) {
                self.classify(ctx, arg, query)
            } else {
                match ctx.semantic.nodes().kind(id) {
                    AstKind::TSInterfaceDeclaration(i)
                        if query == Query::Value
                            && scopes.symbol_declarations(symbol).count() == 1
                            && i.extends.is_empty()
                            && empty(&i.body.body) =>
                    {
                        Class::Broad
                    }
                    AstKind::TSTypeAliasDeclaration(alias) => self.alias(ctx, alias, args, query),
                    _ => Class::None,
                }
            };
            self.visiting.remove(&id);
            return result;
        }
        match name.name.as_str() {
            "Readonly" | "NonNullable" | "Partial" | "Required"
                if matches!(
                    query,
                    Query::Target | Query::AliasTarget | Query::Value | Query::UnsafeDictionary
                ) =>
            {
                args.first()
                    .map_or(Class::None, |v| self.classify(ctx, v, query))
            }
            "Promise" | "PromiseLike" if query == Query::UnknownReturn => args
                .first()
                .map_or(Class::None, |v| self.classify(ctx, v, query)),
            "Record" if query == Query::UnsafeDictionary => args
                .get(1)
                .map_or(Class::None, |v| self.classify(ctx, v, Query::Value)),
            "Pick" | "Omit" if query == Query::UnsafeDictionary => args
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
    fn classify(&mut self, ctx: &TsCtx<'a>, ty: &'a TSType<'a>, query: Query) -> Class {
        let ty = transparent(ty);
        if query == Query::UnsafeDictionary {
            return self.dictionary(ctx, ctx.semantic.nodes().kind(ty.node_id()));
        }
        if let TSType::TSTypeReference(r) = ty {
            return self.reference(ctx, r, query);
        }
        match (ty, query) {
            (TSType::TSObjectKeyword(_), Query::ObjectInput)
            | (TSType::TSUnknownKeyword(_), Query::UnknownReturn | Query::UnknownAlias)
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
            (
                TSType::TSUnionType(n),
                Query::Value
                | Query::Key
                | Query::UnknownReturn
                | Query::UnknownAlias
                | Query::ObjectInput,
            ) if n
                .types
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
    fn dictionary(&mut self, ctx: &TsCtx<'a>, kind: AstKind<'a>) -> Class {
        match kind {
            AstKind::TSTypeReference(r) => self.reference(ctx, r, Query::UnsafeDictionary),
            AstKind::TSTypeLiteral(n) => self.index_values(ctx, &n.members),
            AstKind::TSMappedType(n) => n
                .type_annotation
                .as_ref()
                .map_or(Class::None, |v| self.classify(ctx, v, Query::Value)),
            _ => Class::None,
        }
    }
}

pub(super) fn unsafe_dictionary<'a>(ctx: &TsCtx<'a>, kind: AstKind<'a>) -> bool {
    Resolution::default().dictionary(ctx, kind) != Class::None
}
