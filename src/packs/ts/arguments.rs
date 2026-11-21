use super::{TsCtx, resolve};
use oxc::ast::ast::{Argument, ArrayExpressionElement, Expression};
use oxc::span::{GetSpan, Span};
use std::collections::HashSet;

enum Part<'a> {
    Value(Option<&'a Expression<'a>>),
    Spread(&'a Expression<'a>),
    End(u32, usize),
}

pub(super) struct Slot<'a>(pub(super) Option<&'a Expression<'a>>);

#[derive(Default)]
pub(super) struct Arguments<'a> {
    pending: Vec<Part<'a>>,
    active: HashSet<u32>,
    done: HashSet<u32>,
    emitted: usize,
    unreadable: Option<Span>,
}
impl<'a> Arguments<'a> {
    pub(super) fn prepend(&mut self, args: &'a [Argument<'a>]) {
        self.pending.extend(args.iter().rev().map(|arg| match arg {
            Argument::SpreadElement(s) => Part::Spread(&s.argument),
            a => Part::Value(a.as_expression()),
        }));
    }
    pub(super) fn tagged(values: &'a [Expression<'a>]) -> Self {
        let mut args = Self::default();
        args.pending
            .extend(values.iter().rev().map(|e| Part::Value(Some(e))));
        args.pending.push(Part::Value(None));
        args
    }
    pub(super) fn array(value: Option<&'a Expression<'a>>) -> Self {
        let mut args = Self::default();
        if let Some(value) = value {
            args.pending.push(Part::Spread(value));
        }
        args
    }

    pub(super) fn next(&mut self, ctx: &TsCtx<'a>, distinct: bool) -> Option<Slot<'a>> {
        while let Some(part) = self.pending.pop() {
            match part {
                Part::Value(value) => {
                    self.emitted += 1;
                    return Some(Slot(value));
                }
                Part::End(id, start) => {
                    self.active.remove(&id);
                    if distinct || self.emitted == start {
                        self.done.insert(id);
                    }
                }
                Part::Spread(e) => {
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
                    self.pending.push(Part::End(id, self.emitted));
                    self.pending
                        .extend(array.elements.iter().rev().map(|el| match el {
                            ArrayExpressionElement::SpreadElement(s) => Part::Spread(&s.argument),
                            other => Part::Value(other.as_expression()),
                        }));
                }
            }
        }
        None
    }

    pub(super) fn finish(
        mut self,
        ctx: &TsCtx<'a>,
    ) -> (Vec<Option<&'a Expression<'a>>>, Option<Span>) {
        let mut result = Vec::new();
        // Rules index the first three arguments and search the remaining values.
        while let Some(value) = self.next(ctx, result.len() >= 3) {
            result.push(value.0);
        }
        (result, self.unreadable)
    }
}
