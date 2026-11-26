use crate::{
    engine::TsCtx,
    packs::ts::values::{is_written, symbol, unwrapped},
};
use oxc::{
    ast::{
        AstKind,
        ast::{BindingPattern, Expression, IdentifierReference, ObjectPattern, PropertyKey},
    },
    syntax::symbol::SymbolId,
};
use std::{
    cell::RefCell,
    collections::{HashMap, HashSet},
    rc::Rc,
};

pub(super) struct Values<'ctx, 'a> {
    ctx: &'ctx TsCtx<'a>,
    aliases: RefCell<HashMap<SymbolId, &'a Expression<'a>>>,
    patterns: RefCell<HashMap<oxc::span::Span, Rc<PatternIndex<'a>>>>,
}

impl<'ctx, 'a> Values<'ctx, 'a> {
    pub fn new(ctx: &'ctx TsCtx<'a>) -> Self {
        Self {
            ctx,
            aliases: RefCell::default(),
            patterns: RefCell::default(),
        }
    }

    pub fn declaration(&self, id: &IdentifierReference<'_>) -> Option<AstKind<'a>> {
        self.declaration_for_symbol(symbol(self.ctx, id)?)
    }

    pub fn declaration_for_symbol(&self, symbol: SymbolId) -> Option<AstKind<'a>> {
        (!is_written(self.ctx, symbol)
            && self
                .ctx
                .semantic
                .scoping()
                .symbol_redeclarations(symbol)
                .is_empty())
        .then(|| self.ctx.semantic.symbol_declaration(symbol).kind())
    }

    pub fn initializer(&self, id: &IdentifierReference<'_>) -> Option<&'a Expression<'a>> {
        match self.declaration(id)? {
            AstKind::VariableDeclarator(v)
                if matches!(v.id, BindingPattern::BindingIdentifier(_)) =>
            {
                v.init.as_ref()
            }
            _ => None,
        }
    }

    pub fn resolve(&self, mut expr: &'a Expression<'a>) -> &'a Expression<'a> {
        let mut seen = HashSet::new();
        let mut aliases = Vec::new();
        loop {
            expr = unwrapped(self.ctx, expr);
            let Expression::Identifier(id) = expr else {
                break;
            };
            if let Some(sym) = symbol(self.ctx, id)
                && let Some(&cached) = self.aliases.borrow().get(&sym)
            {
                expr = cached;
                break;
            }
            if !seen.insert(id.span.start) {
                return expr;
            }
            let Some(next) = self.initializer(id) else {
                break;
            };
            aliases.extend(symbol(self.ctx, id));
            expr = next;
        }
        self.aliases
            .borrow_mut()
            .extend(aliases.into_iter().map(|s| (s, expr)));
        expr
    }

    pub fn literal(&self, expr: &'a Expression<'a>) -> Option<&'a str> {
        match self.resolve(expr) {
            Expression::StringLiteral(s) => s.value.as_str(),
            Expression::TemplateLiteral(t) if t.expressions.is_empty() => {
                t.quasis.first()?.value.cooked?.as_str()
            }
            _ => None,
        }
    }

    pub fn key(&self, key: &'a PropertyKey<'a>) -> Option<&'a str> {
        if let PropertyKey::StaticIdentifier(id) = key {
            return Some(id.name.as_str());
        }
        let expr = key.as_expression()?;
        if let Expression::NumericLiteral(n) = expr {
            return n.raw.as_ref().map(oxc::str::Str::as_str);
        }
        self.literal(expr)
    }

    fn pattern_index(&self, pattern: &'a ObjectPattern<'a>) -> Rc<PatternIndex<'a>> {
        if let Some(hit) = self.patterns.borrow().get(&pattern.span) {
            return Rc::clone(hit);
        }
        let mut index = PatternIndex {
            members: HashMap::new(),
            simple: pattern.rest.is_none(),
        };
        for prop in self.ctx.counted(pattern.properties.iter()) {
            let name = self.key(&prop.key);
            index.simple &=
                matches!(prop.value, BindingPattern::BindingIdentifier(_)) && name.is_some();
            if let Some(symbol) = prop
                .value
                .get_binding_identifier()
                .and_then(|b| b.symbol_id.get())
            {
                index.members.entry(symbol).or_insert(name);
            }
        }
        let index = Rc::new(index);
        self.patterns
            .borrow_mut()
            .insert(pattern.span, Rc::clone(&index));
        index
    }

    // A load that looks allowed must not rest on a guess.
    pub fn simple_pattern(&self, pattern: &'a ObjectPattern<'a>) -> bool {
        self.pattern_index(pattern).simple
    }

    pub fn bound_member(
        &self,
        id: &IdentifierReference<'a>,
    ) -> Option<(&'a Expression<'a>, &'a str)> {
        let AstKind::VariableDeclarator(v) = self.declaration(id)? else {
            return None;
        };
        let BindingPattern::ObjectPattern(pattern) = &v.id else {
            return None;
        };
        Some((
            v.init.as_ref()?,
            self.pattern_index(pattern)
                .members
                .get(&symbol(self.ctx, id)?)
                .copied()
                .flatten()?,
        ))
    }
}

struct PatternIndex<'a> {
    members: HashMap<SymbolId, Option<&'a str>>,
    simple: bool,
}
