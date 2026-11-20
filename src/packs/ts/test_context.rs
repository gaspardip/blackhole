use super::values::MemberKeys;
use super::values::{
    Invocation, Origin, Properties, binding_value, call_expr, declaration, initializer, invoked,
    origin_path, properties, resolve, resolves_to_global, symbol, unwrapped,
};
use crate::engine::TsCtx;
use oxc::ast::{
    AstKind,
    ast::{
        ArrowFunctionBody, BindingPattern, Expression, FunctionBody, Statement, TSFunctionType,
        TSType, TSTypeAnnotation,
    },
};
use oxc::span::GetSpan;
use oxc::syntax::operator::BinaryOperator;

pub(super) fn test_file(path: &str) -> bool {
    let mut parts = path.rsplit(['/', '\\']);
    let file = parts.next().unwrap_or_default();
    file.split('.').any(|part| {
        matches!(
            part,
            "test" | "spec" | "fixture" | "mock" | "setup" | "commands"
        )
    }) || parts.any(|part| {
        matches!(
            part,
            "__tests__" | "test" | "tests" | "testing" | "test-utils"
        )
    })
}

pub(super) fn is_test(name: &str) -> bool {
    matches!(name, "it" | "test" | "specify")
}
fn single_statement<'a>(body: &'a FunctionBody<'a>) -> Option<&'a Statement<'a>> {
    if !body.directives.is_empty() {
        return None;
    }
    let [statement] = body.statements.as_slice() else {
        return None;
    };
    Some(statement)
}
fn callback_value<'a>(
    callback: &'a Expression<'a>,
    statement: fn(&'a Statement<'a>) -> Option<&'a Expression<'a>>,
) -> Option<&'a Expression<'a>> {
    let body = match callback {
        Expression::FunctionExpression(f) => f.body.as_deref()?,
        Expression::ArrowFunctionExpression(f) => match &f.body {
            ArrowFunctionBody::FunctionBody(b) => b,
            other => return other.as_expression(),
        },
        _ => return None,
    };
    statement(single_statement(body)?)
}
fn expression_statement<'a>(s: &'a Statement<'a>) -> Option<&'a Expression<'a>> {
    match s {
        Statement::ExpressionStatement(s) => Some(&s.expression),
        _ => None,
    }
}
pub(super) fn single_effect<'a>(callback: &'a Expression<'a>) -> Option<&'a Expression<'a>> {
    callback_value(callback, expression_statement)
}
pub(super) fn single_value<'a>(callback: &'a Expression<'a>) -> Option<&'a Expression<'a>> {
    callback_value(callback, |s| match s {
        Statement::ReturnStatement(r) => r.argument.as_ref(),
        _ => expression_statement(s),
    })
}
pub(super) fn suite(name: &str) -> bool {
    matches!(name, "describe" | "suite" | "context")
}

fn modifier(name: &str) -> bool {
    matches!(
        name,
        "skip" | "only" | "todo" | "skipIf" | "runIf" | "fixme" | "fails" | "failing"
    )
}

#[derive(Clone, Copy, Default, PartialEq)]
enum ModuleRoot {
    #[default]
    None,
    DefaultTest,
    CallableTest,
}

#[derive(Clone, Copy, Default, PartialEq)]
enum OptionSyntax {
    #[default]
    Named,
    Flexible,
    Node,
    Playwright,
}

#[derive(Clone, Copy, Default)]
pub(crate) struct Runner<'a> {
    prefix: [&'a str; 3],
    length: usize,
    module_root: ModuleRoot,
    option_syntax: OptionSyntax,
    pub(super) last: Option<&'a str>,
    pub(super) skipped: bool,
    pub(super) skip_at: Option<usize>,
    pub(super) todo: bool,
}
impl<'a> Runner<'a> {
    pub(super) fn from_path(origin: Origin, parts: &[&'a str]) -> Option<Self> {
        let mut result = Self {
            option_syntax: match (origin, parts.first().copied()) {
                (Origin::Global, _) | (_, Some("vitest")) => OptionSyntax::Flexible,
                (_, Some("node:test")) => OptionSyntax::Node,
                (_, Some("@playwright/test")) => OptionSyntax::Playwright,
                _ => OptionSyntax::Named,
            },
            ..Self::default()
        };
        let names = match (origin, parts) {
            (Origin::Global, names)
            | (_, names @ ["node:timers/promises" | "timers/promises", ..]) => names,
            (Origin::Required, ["node:test", rest @ ..]) => {
                result.push("test");
                result.module_root = ModuleRoot::CallableTest;
                rest
            }
            (_, ["node:test" | "@playwright/test", rest @ ..]) => {
                result.module_root = ModuleRoot::DefaultTest;
                rest
            }
            (_, ["vitest" | "@jest/globals", rest @ ..]) => rest,
            (
                _,
                [
                    "node:assert" | "node:assert/strict" | "assert" | "assert/strict",
                    rest @ ..,
                ],
            ) => {
                result.push("assert");
                rest.strip_prefix(&["default"]).unwrap_or(rest)
            }
            _ => return None,
        };
        for name in names {
            result.push(name);
        }
        Some(result)
    }

    fn push(&mut self, name: &'a str) {
        let root = std::mem::take(&mut self.module_root);
        if root == ModuleRoot::CallableTest
            && matches!(
                name,
                "default"
                    | "test"
                    | "it"
                    | "describe"
                    | "suite"
                    | "before"
                    | "after"
                    | "beforeEach"
                    | "afterEach"
                    | "mock"
                    | "assert"
                    | "run"
            )
        {
            self.length = 0;
        }
        let name = if root != ModuleRoot::None && name == "default" {
            "test"
        } else {
            name
        };
        if let Some(slot) = self.prefix.get_mut(self.length) {
            *slot = name;
        }
        self.skipped |= self.length > 0 && modifier(name);
        self.length += 1;
        self.last = Some(name);
    }
    pub(super) fn short(&self) -> Option<&[&'a str]> {
        self.prefix.get(..self.length)
    }
    pub(super) fn first(&self) -> Option<&&'a str> {
        (self.length > 0).then_some(&self.prefix[0])
    }
    pub(super) fn is_empty(&self) -> bool {
        self.length == 0
    }
}
impl<const N: usize> PartialEq<[&str; N]> for Runner<'_> {
    fn eq(&self, other: &[&str; N]) -> bool {
        self.short() == Some(other.as_slice())
    }
}

pub(super) fn runner<'a>(ctx: &TsCtx<'a>, call: &Invocation<'a>) -> Runner<'a> {
    let mut e = call.callee;
    let mut steps = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let mut result = loop {
        e = resolve(ctx, e);
        let span = e.span();
        if let Some(result) = ctx.resolution.runners.borrow().get(&span) {
            break *result;
        }
        if !seen.insert(span) {
            return Runner::default();
        }
        if let Some(m) = e.get_member_expr()
            && let Some(key) = MemberKeys(ctx).read(m)
        {
            let at = super::values::member_span(ctx, m).start as usize;
            steps.push((span, Some((key, at))));
            e = m.object();
            continue;
        }
        if let Some((origin, parts)) = origin_path(ctx, e) {
            let Some(result) = Runner::from_path(origin, &parts) else {
                return Runner::default();
            };
            ctx.resolution.runners.borrow_mut().insert(span, result);
            break result;
        }
        if let Some(c) = call_expr(e) {
            steps.push((span, None));
            e = invoked(ctx, c).callee;
            continue;
        }
        if let Expression::TaggedTemplateExpression(t) = e {
            steps.push((span, None));
            e = &t.tag;
            continue;
        }
        return Runner::default();
    };
    let mut cache = ctx.resolution.runners.borrow_mut();
    for (span, key) in steps.into_iter().rev() {
        if let Some((key, at)) = key {
            if modifier(key) {
                result.skip_at = Some(at);
            }
            result.todo |= key == "todo";
            result.push(key);
        }
        cache.insert(span, result);
    }
    result
}

impl Runner<'_> {
    pub(super) fn test(&self) -> bool {
        self.first().is_some_and(|name| is_test(name))
    }
    pub(super) fn test_or_suite(&self) -> bool {
        self.test() || self.first().is_some_and(|name| suite(name))
    }

    pub(super) fn legacy_skip(&self) -> bool {
        matches!(
            self.first().copied(),
            Some("xit" | "xtest" | "xdescribe" | "fit" | "fdescribe")
        )
    }

    pub(super) fn is_node(&self) -> bool {
        self.option_syntax == OptionSyntax::Node
    }

    pub(super) fn is_playwright(&self) -> bool {
        self.option_syntax == OptionSyntax::Playwright
    }
}

pub(super) enum TestOption<'a> {
    Object(&'a Expression<'a>, Properties<'a>),
    Opaque { at: usize, before_callback: bool },
    Timeout(usize),
}

fn declared_signature(decl: AstKind<'_>) -> Option<&TSFunctionType<'_>> {
    let annotation = match decl {
        AstKind::FormalParameter(p) => p.type_annotation.as_deref(),
        AstKind::VariableDeclarator(v) => v.type_annotation.as_deref(),
        _ => None,
    }?;
    match annotation.type_annotation.without_parenthesized() {
        TSType::TSFunctionType(f) => Some(f),
        _ => None,
    }
}

fn declared_callback(ctx: &TsCtx<'_>, expr: &Expression<'_>) -> bool {
    expr.get_identifier_reference()
        .and_then(|id| declaration(ctx, id))
        .is_some_and(|decl| {
            matches!(decl, AstKind::Function(_)) || declared_signature(decl).is_some()
        })
}

fn callback<'a>(ctx: &TsCtx<'a>, mut expr: &'a Expression<'a>, syntax: OptionSyntax) -> bool {
    let mut seen = std::collections::HashSet::new();
    loop {
        let value = resolve(ctx, expr);
        if value.is_function() || declared_callback(ctx, value) || declared_callback(ctx, expr) {
            return true;
        }
        let Some(member) = call_expr(value)
            .and_then(|call| resolve(ctx, &call.callee).get_member_expr())
            .filter(|m| MemberKeys(ctx).read(m) == Some("bind"))
        else {
            return false;
        };
        if !seen.insert(value.span()) {
            return false;
        }
        expr = member.object();
        if syntax == OptionSyntax::Node
            && origin_path(ctx, expr).is_some_and(|(origin, path)| {
                origin == Origin::Module
                    && matches!(
                        path.split_first(),
                        Some((
                            &("node:assert" | "node:assert/strict" | "assert" | "assert/strict"),
                            ["equal"] | ["default" | "strict", "equal"]
                        ))
                    )
            })
        {
            return true;
        }
    }
}

fn string_type(annotation: &TSTypeAnnotation<'_>) -> bool {
    matches!(
        annotation.type_annotation.without_parenthesized(),
        TSType::TSStringKeyword(_)
    )
}

fn annotated_string_title<'a>(ctx: &TsCtx<'a>, mut expr: &'a Expression<'a>) -> bool {
    let mut seen = std::collections::HashSet::new();
    let mut annotated = false;
    loop {
        let value = unwrapped(expr);
        let Some(id) = value.get_identifier_reference() else {
            return annotated && value.is_function();
        };
        if !seen.insert(id.span) {
            return false;
        }
        let Some(decl) = declaration(ctx, id) else {
            return annotated && symbol(ctx, id).is_none();
        };
        let init = match decl {
            AstKind::VariableDeclarator(v)
                if matches!(v.id, BindingPattern::BindingIdentifier(_)) =>
            {
                v.init.as_ref()
            }
            AstKind::FormalParameter(p)
                if matches!(p.pattern, BindingPattern::BindingIdentifier(_)) =>
            {
                p.initializer.as_deref()
            }
            AstKind::Function(_)
            | AstKind::ImportSpecifier(_)
            | AstKind::ImportDefaultSpecifier(_)
            | AstKind::ImportNamespaceSpecifier(_) => return annotated,
            _ => return false,
        };
        annotated |= declared_signature(decl).is_some_and(|f| string_type(&f.return_type));
        let Some(init) = init else {
            return annotated;
        };
        if !annotated
            && matches!(decl, AstKind::FormalParameter(_))
            && initializer(ctx, id).is_none()
        {
            return false;
        }
        expr = init;
    }
}

fn returns_string<'a>(ctx: &TsCtx<'a>, expr: &'a Expression<'a>) -> bool {
    let Some(call) = call_expr(expr) else {
        return false;
    };
    if annotated_string_title(ctx, &call.callee) {
        return true;
    }
    let function = match binding_value(ctx, &call.callee) {
        Expression::Identifier(id) => declaration(ctx, id),
        Expression::FunctionExpression(f) => Some(AstKind::Function(f)),
        Expression::ArrowFunctionExpression(f) => Some(AstKind::ArrowFunctionExpression(f)),
        _ => None,
    };
    function
        .filter(|kind| match kind {
            AstKind::Function(f) => !f.r#async && !f.generator,
            AstKind::ArrowFunctionExpression(f) => !f.r#async,
            _ => false,
        })
        .and_then(super::function_types::returns)
        .is_some_and(string_type)
}

fn string_title<'a>(ctx: &TsCtx<'a>, arg: &'a Expression<'a>) -> bool {
    let mut pending = vec![arg];
    let mut seen = std::collections::HashSet::new();
    while let Some(expr) = pending.pop() {
        if let Some(id) = unwrapped(expr).get_identifier_reference()
            && let Some(kind @ (AstKind::VariableDeclarator(_) | AstKind::FormalParameter(_))) =
                declaration(ctx, id)
            && super::function_types::direct_binding(kind)
                .and_then(|binding| binding.annotation)
                .is_some_and(string_type)
        {
            return true;
        }
        let value = resolve(ctx, expr);
        if !seen.insert(value.span()) {
            continue;
        }
        match value {
            Expression::StringLiteral(_) | Expression::TemplateLiteral(_) => return true,
            Expression::BinaryExpression(b) if b.operator == BinaryOperator::Addition => {
                pending.extend([&b.right, &b.left]);
            }
            _ if call_expr(value)
                .is_some_and(|call| resolves_to_global(ctx, &call.callee, "String")) =>
            {
                return true;
            }
            _ if returns_string(ctx, value) => return true,
            _ => (),
        }
    }
    false
}

impl<'a> Runner<'a> {
    pub(super) fn test_options(
        self,
        ctx: &TsCtx<'a>,
        call: &Invocation<'a>,
    ) -> [Option<TestOption<'a>>; 2] {
        let object = |arg| properties(ctx, arg).map(|props| TestOption::Object(arg, props));
        let is_callback = |arg| callback(ctx, arg, self.option_syntax);
        let opaque = |arg| {
            let value = resolve(ctx, arg);
            !value.is_literal()
                && !matches!(value, Expression::TemplateLiteral(_))
                && !resolves_to_global(ctx, value, "undefined")
                && !is_callback(arg)
        };
        let [first, second, third] = [call.arg(0), call.arg(1), call.arg(2)];
        if (self.option_syntax == OptionSyntax::Node
            || self.option_syntax == OptionSyntax::Flexible && self.length == 1)
            && let Some(first) = first
        {
            if let Some(options) = object(first) {
                return [Some(options), None];
            }
            if self.option_syntax == OptionSyntax::Node
                && third.is_none()
                && second.is_some_and(is_callback)
                && opaque(first)
                && !string_title(ctx, first)
            {
                return [
                    Some(TestOption::Opaque {
                        at: first.span().start as usize,
                        before_callback: true,
                    }),
                    None,
                ];
            }
        }
        let mut options = second.and_then(object);
        if options.is_none()
            && matches!(
                self.option_syntax,
                OptionSyntax::Node | OptionSyntax::Flexible
            )
        {
            let before_callback = third.is_some_and(is_callback);
            if let Some(arg) = second
                && (self.length == 1 || before_callback)
                && opaque(arg)
            {
                options = Some(TestOption::Opaque {
                    at: arg.span().start as usize,
                    before_callback,
                });
            } else if self.length == 1
                && call.args.len() <= 1
                && let Some(spread) = call.unreadable_args
            {
                options = Some(TestOption::Opaque {
                    at: spread.start as usize,
                    before_callback: false,
                });
            }
        }
        let timeout = third.and_then(|arg| {
            if matches!(
                self.option_syntax,
                OptionSyntax::Node | OptionSyntax::Playwright
            ) {
                return None;
            }
            object(arg).or_else(|| {
                (second.is_some()
                    && !matches!(options, Some(TestOption::Object(..)))
                    && !is_callback(arg))
                .then_some(TestOption::Timeout(arg.span().start as usize))
            })
        });
        [options, timeout]
    }
}

impl Invocation<'_> {
    pub(super) fn inline_test_body(&self) -> Option<oxc::span::Span> {
        self.args
            .iter()
            .flatten()
            .find_map(|arg| match super::values::unwrapped(arg) {
                Expression::ArrowFunctionExpression(f) => Some(f.body.span()),
                Expression::FunctionExpression(f) => Some(f.body.as_ref()?.span),
                _ => None,
            })
    }
}
pub(super) fn enclosing_runners<'ctx, 'a>(
    ctx: &'ctx TsCtx<'a>,
    id: oxc::syntax::node::NodeId,
) -> impl Iterator<Item = Runner<'a>> + 'ctx {
    super::values::enclosing_functions(ctx, id).map(|f| {
        super::values::enclosing_call(ctx, f.id())
            .map_or_else(Runner::default, |call| runner(ctx, &call))
    })
}

pub(super) fn test_callback_parameter<'a>(
    ctx: &TsCtx<'a>,
    symbol: oxc::syntax::symbol::SymbolId,
) -> Option<(&'a oxc::ast::ast::FormalParameter<'a>, Runner<'a>)> {
    let declaration = ctx.semantic.symbol_declaration(symbol);
    let function = super::values::enclosing_functions(ctx, declaration.id()).next()?;
    let params = match function.kind() {
        AstKind::Function(f) => &f.params,
        AstKind::ArrowFunctionExpression(f) => &f.params,
        _ => return None,
    };
    let parameter = params.items.first()?;
    if !parameter
        .pattern
        .any_binding_identifiers(&mut |id| id.symbol_id.get() == Some(symbol))
    {
        return None;
    }
    let call = super::values::enclosing_call(ctx, function.id())?;
    let runner = runner(ctx, &call);
    (runner.test()
        && call
            .args
            .iter()
            .flatten()
            .any(|arg| resolve(ctx, arg).span() == function.span()))
    .then_some((parameter, runner))
}
