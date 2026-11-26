use super::function_types::direct_binding;
use super::values::{
    InvocationTarget, MemberKeys, Origin, ThisArg, call_expr, declaration, invocation_target,
    member_values, origin_path, path_is, receiver, resolve, resolves_to_global, same_key, symbol,
    unwrapped,
};
use crate::engine::TsCtx;
use oxc::{
    ast::{
        AstKind,
        ast::{Expression, IdentifierReference, TSType, TSTypeName, TSTypeOperatorOperator},
    },
    span::{GetSpan, Span},
    syntax::symbol::SymbolId,
};
use std::collections::{HashMap, HashSet};

#[derive(Clone, Copy)]
pub(super) enum ArrayType {
    Array,
    Union,
    Other,
}

#[derive(Default)]
pub(super) struct ArrayEvidence {
    expressions: HashMap<Span, bool>,
    aliases: HashMap<SymbolId, ArrayType>,
}

impl ArrayEvidence {
    pub(super) fn array_type<'a>(&mut self, ctx: &TsCtx<'a>, mut ty: &'a TSType<'a>) -> ArrayType {
        let mut path = Vec::new();
        let result = loop {
            if !ctx.resolution.take_work(1) {
                return ArrayType::Other;
            }
            match ty {
                TSType::TSParenthesizedType(p) => ty = &p.type_annotation,
                TSType::TSArrayType(_) | TSType::TSTupleType(_) => break ArrayType::Array,
                TSType::TSUnionType(_) => break ArrayType::Union,
                TSType::TSTypeOperatorType(t) if t.operator == TSTypeOperatorOperator::Readonly => {
                    ty = &t.type_annotation;
                }
                TSType::TSTypeReference(t) => {
                    let TSTypeName::IdentifierReference(name) = &t.type_name else {
                        break ArrayType::Other;
                    };
                    let Some(id) = symbol(ctx, name) else {
                        let name = name.name.as_str();
                        break if same_key(ctx, name, "Array")
                            || same_key(ctx, name, "ReadonlyArray")
                        {
                            ArrayType::Array
                        } else {
                            ArrayType::Other
                        };
                    };
                    if let Some(&known) = self.aliases.get(&id) {
                        break known;
                    }
                    let Some(AstKind::TSTypeAliasDeclaration(alias)) = sole_declaration(ctx, id)
                    else {
                        break ArrayType::Other;
                    };
                    self.aliases.insert(id, ArrayType::Other);
                    path.push(id);
                    ty = &alias.type_annotation;
                }
                _ => break ArrayType::Other,
            }
        };
        self.aliases.extend(path.into_iter().map(|id| (id, result)));
        result
    }

    pub(super) fn native_reduce<'a>(
        &mut self,
        ctx: &TsCtx<'a>,
        target: &InvocationTarget<'a>,
    ) -> bool {
        let Some((owner, key)) = receiver(ctx, target.callee) else {
            return false;
        };
        if !same_key(ctx, key, "reduce") && !same_key(ctx, key, "reduceRight") {
            return false;
        }
        match target.this_arg {
            ThisArg::Member => self.known(ctx, owner),
            ThisArg::Explicit(Some(arg)) => {
                (target
                    .global_path()
                    .is_some_and(|path| path_is(ctx, path, &["Array", "prototype", key]))
                    || self.known(ctx, owner))
                    && self.known(ctx, arg)
            }
            ThisArg::Explicit(None) => false,
        }
    }

    pub(super) fn known<'a>(&mut self, ctx: &TsCtx<'a>, mut expr: &'a Expression<'a>) -> bool {
        let mut path = Vec::new();
        let result = loop {
            if !ctx.resolution.take_work(1) {
                return false;
            }
            expr = unwrapped(ctx, expr);
            if !ctx.resolution.has_work() {
                return false;
            }
            let span = expr.span();
            if let Some(answer) = self.expressions.get(&span) {
                break *answer;
            }
            // Revisiting a pending prefix means the chain has no array origin.
            self.expressions.insert(span, false);
            path.push(span);
            match expr {
                Expression::ArrayExpression(_) => break true,
                Expression::NewExpression(new) => {
                    break origin_path(ctx, &new.callee).is_some_and(|(origin, path)| {
                        origin == Origin::Global && path_is(ctx, &path, &["Array"])
                    });
                }
                Expression::Identifier(id) => {
                    let Some(kind) = declaration(ctx, id) else {
                        // TypeScript keeps every write to an annotated binding within its type.
                        break annotated(ctx, id).is_some_and(|annotation| {
                            matches!(self.array_type(ctx, annotation), ArrayType::Array)
                        });
                    };
                    let Some(binding) = direct_binding(kind) else {
                        break false;
                    };
                    // A rest parameter holds an array whatever its annotation says.
                    if matches!(kind, AstKind::FormalParameterRest(_)) {
                        break true;
                    }
                    if let Some(annotation) = binding.annotation {
                        match self.array_type(ctx, &annotation.type_annotation) {
                            ArrayType::Array => break true,
                            ArrayType::Union => break false,
                            ArrayType::Other => {}
                        }
                    }
                    let Some(next) = binding.initializer else {
                        break false;
                    };
                    expr = next;
                }
                _ => {
                    // The identifier arm still needs to see the binding's annotation.
                    if let Some(value) = expr
                        .get_member_expr()
                        .and_then(|m| member_values::value(ctx, m))
                        && unwrapped(ctx, value).span() != span
                    {
                        expr = value;
                        continue;
                    }
                    let Some(node) = call_expr(expr) else {
                        break false;
                    };
                    let Some(call) = invocation_target(ctx, node) else {
                        break false;
                    };
                    if let Some(path) = call.global_path() {
                        if path_is(ctx, path, &["Array"]) {
                            break true;
                        }
                        if path_is(ctx, path, &["Array", "from"])
                            || path_is(ctx, path, &["Array", "of"])
                        {
                            break call.owner_constructs(ctx);
                        }
                    }
                    let Some(receiver) =
                        call.method(ctx, |name| returns_array(ctx, name), |e| self.known(ctx, e))
                    else {
                        break false;
                    };
                    expr = receiver;
                }
            }
        };
        for span in path {
            self.expressions.insert(span, result);
        }
        result
    }
}

fn returns_array(ctx: &TsCtx<'_>, name: &str) -> bool {
    [
        "map",
        "filter",
        "flat",
        "flatMap",
        "slice",
        "concat",
        "splice",
        "reverse",
        "sort",
        "fill",
        "copyWithin",
        "toSorted",
        "toReversed",
        "toSpliced",
        "with",
    ]
    .into_iter()
    .any(|method| same_key(ctx, name, method))
}

fn annotated<'a>(ctx: &TsCtx<'a>, id: &IdentifierReference<'_>) -> Option<&'a TSType<'a>> {
    let binding = direct_binding(sole_declaration(ctx, symbol(ctx, id)?)?)?;
    Some(&binding.annotation?.type_annotation)
}

pub(super) fn sole_declaration<'a>(ctx: &TsCtx<'a>, symbol: SymbolId) -> Option<AstKind<'a>> {
    // A name shared with a value is ambiguous.
    ctx.semantic
        .scoping()
        .symbol_redeclarations(symbol)
        .is_empty()
        .then(|| ctx.semantic.symbol_declaration(symbol).kind())
}

fn same_receiver<'a>(
    ctx: &TsCtx<'a>,
    mut left: &'a Expression<'a>,
    mut right: &'a Expression<'a>,
) -> bool {
    let mut seen = HashSet::new();
    loop {
        if !ctx.resolution.take_work(1) {
            return false;
        }
        left = resolve(ctx, left);
        right = resolve(ctx, right);
        if !ctx.resolution.has_work() || !seen.insert((left.span(), right.span())) {
            return false;
        }
        if left.span() == right.span() {
            return true;
        }
        if let (Expression::Identifier(l), Expression::Identifier(r)) = (left, right) {
            return match (symbol(ctx, l), symbol(ctx, r)) {
                (Some(left), Some(right)) => left == right,
                (None, None) => same_key(ctx, l.name.as_str(), r.name.as_str()),
                _ => false,
            };
        }
        let (Some(l), Some(r)) = (left.get_member_expr(), right.get_member_expr()) else {
            return false;
        };
        let keys = MemberKeys(ctx);
        let Some(l_key) = keys.read(l) else {
            return false;
        };
        let Some(r_key) = keys.read(r) else {
            return false;
        };
        if !same_key(ctx, l_key, r_key) {
            return false;
        }
        left = l.object();
        right = r.object();
    }
}

impl<'a> InvocationTarget<'a> {
    pub(super) fn method(
        &self,
        ctx: &TsCtx<'a>,
        matches_name: impl FnOnce(&str) -> bool,
        is_array: impl FnOnce(&'a Expression<'a>) -> bool,
    ) -> Option<&'a Expression<'a>> {
        let (owner, key) = receiver(ctx, self.callee)?;
        if !matches_name(key) {
            return None;
        }
        match self.this_arg {
            ThisArg::Member => Some(owner),
            ThisArg::Explicit(Some(arg))
                if self.global_path().is_some_and(|path| {
                    let [root, prototype, method] = path else {
                        return false;
                    };
                    same_key(ctx, root, "Array")
                        && same_key(ctx, prototype, "prototype")
                        && same_key(ctx, method, key)
                }) || same_receiver(ctx, owner, arg)
                    || is_array(owner) =>
            {
                Some(arg)
            }
            ThisArg::Explicit(_) => None,
        }
    }
    // Array factories accept a constructor as their receiver.
    fn owner_constructs(&self, ctx: &TsCtx<'a>) -> bool {
        let arg = match self.this_arg {
            ThisArg::Member => return true,
            ThisArg::Explicit(None) => return false,
            ThisArg::Explicit(Some(arg)) => arg,
        };
        let arg = resolve(ctx, arg);
        ctx.resolution.has_work()
            && (arg.is_literal()
                || arg.is_void_0()
                || resolves_to_global(ctx, arg, "undefined")
                || origin_path(ctx, arg).is_some_and(|(origin, path)| {
                    origin == Origin::Global && path_is(ctx, &path, &["Array"])
                }))
    }
}
