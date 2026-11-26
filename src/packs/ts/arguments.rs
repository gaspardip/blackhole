use super::{TsCtx, resolve};
use oxc::ast::ast::{Argument, ArrayExpressionElement, Expression};
use oxc::span::{GetSpan, Span};
use std::collections::HashSet;

#[derive(Clone, Copy)]
enum Spelling {
    Positional,
    Spread,
}

enum Part<'a> {
    Value(Option<&'a Expression<'a>>, Spelling),
    Array(&'a Expression<'a>, Spelling),
    End(u32, usize),
}

struct Slot<'a>(Option<&'a Expression<'a>>);

pub(super) struct ExpandedArguments<'a> {
    pub(super) values: Vec<Option<&'a Expression<'a>>>,
    pub(super) unreadable: Option<Span>,
    pub(super) positional: usize,
}

#[derive(Default)]
pub(super) struct Arguments<'a> {
    pending: Vec<Part<'a>>,
    active: HashSet<u32>,
    done: HashSet<u32>,
    emitted: usize,
    unreadable: Option<Span>,
    first_spread: Option<usize>,
}
impl<'a> Arguments<'a> {
    pub(super) fn prepend(&mut self, args: &'a [Argument<'a>]) {
        self.pending.extend(args.iter().rev().map(|arg| match arg {
            Argument::SpreadElement(s) => Part::Array(&s.argument, Spelling::Spread),
            a => Part::Value(a.as_expression(), Spelling::Positional),
        }));
    }

    pub(super) fn new(values: &'a [Argument<'a>]) -> Self {
        let mut args = Self::default();
        args.prepend(values);
        args
    }
    pub(super) fn tagged(values: &'a [Expression<'a>]) -> Self {
        let mut args = Self::default();
        args.pending.extend(
            values
                .iter()
                .rev()
                .map(|e| Part::Value(Some(e), Spelling::Positional)),
        );
        args.pending.push(Part::Value(None, Spelling::Positional));
        args
    }
    pub(super) fn array(value: Option<&'a Expression<'a>>) -> Self {
        let mut args = Self::default();
        if let Some(value) = value {
            args.pending.push(Part::Array(value, Spelling::Positional));
        }
        args
    }

    pub(super) fn next_expression(&mut self, ctx: &TsCtx<'a>) -> Option<&'a Expression<'a>> {
        self.next(ctx, false).and_then(|slot| slot.0)
    }

    fn next(&mut self, ctx: &TsCtx<'a>, distinct: bool) -> Option<Slot<'a>> {
        while ctx.take_work(1)
            && let Some(part) = self.pending.pop()
        {
            match part {
                Part::Value(value, spelling) => {
                    self.record_spread(spelling);
                    self.emitted += 1;
                    return Some(Slot(value));
                }
                Part::End(id, start) => {
                    self.active.remove(&id);
                    if distinct || self.emitted == start {
                        self.done.insert(id);
                    }
                }
                Part::Array(e, spelling) => {
                    self.record_spread(spelling);
                    let Expression::ArrayExpression(array) = resolve(ctx, e) else {
                        self.unreadable = Some(e.span());
                        self.pending.clear();
                        return None;
                    };
                    let id = array.span.start;
                    if self.done.contains(&id) {
                        continue;
                    }
                    if !self.active.insert(id) {
                        self.unreadable = Some(e.span());
                        self.pending.clear();
                        return None;
                    }
                    if !ctx.take_work(array.elements.len()) {
                        return None;
                    }
                    self.pending.push(Part::End(id, self.emitted));
                    self.pending
                        .extend(array.elements.iter().rev().map(|el| match el {
                            ArrayExpressionElement::SpreadElement(s) => {
                                Part::Array(&s.argument, Spelling::Spread)
                            }
                            other => Part::Value(other.as_expression(), spelling),
                        }));
                }
            }
        }
        None
    }

    fn record_spread(&mut self, spelling: Spelling) {
        if matches!(spelling, Spelling::Spread) {
            self.first_spread.get_or_insert(self.emitted);
        }
    }

    pub(super) fn finish(mut self, ctx: &TsCtx<'a>) -> ExpandedArguments<'a> {
        let start = self.emitted;
        // Wrapper operands already consumed by invocation resolution are outside the argument list.
        self.first_spread = None;
        let mut values = Vec::new();
        // Rules index the first three arguments and search the remaining values.
        while let Some(value) = self.next(ctx, values.len() >= 3) {
            values.push(value.0);
        }
        let positional = self.first_spread.map_or(values.len(), |i| i - start);
        ExpandedArguments {
            values,
            unreadable: self.unreadable,
            positional,
        }
    }
}
