use super::{
    native_properties::protected,
    values::{declaration, key, literal, member_key, path, resolve},
};
use crate::engine::TsCtx;
use oxc::{
    ast::{
        AstKind,
        ast::{
            Argument, AssignmentTarget, BindingPattern, CallExpression, Expression,
            MemberExpression, ObjectPropertyKind,
        },
    },
    span::GetSpan,
};
use std::collections::HashSet;

fn native(ctx: &TsCtx<'_>, expr: &Expression<'_>) -> Option<String> {
    let parts = path(ctx, expr)?;
    Some(native_name(&parts))
}
fn native_name(parts: &[&str]) -> String {
    let skip = usize::from(
        parts
            .first()
            .is_some_and(|s| matches!(*s, "window" | "globalThis" | "global" | "self")),
    );
    parts[skip..].join(".")
}

fn protected_member(ctx: &TsCtx<'_>, m: &MemberExpression<'_>) -> bool {
    native(ctx, m.object())
        .zip(member_key(ctx, m))
        .is_some_and(|(owner, k)| protected(&owner, k))
}

pub(super) fn writes(ctx: &TsCtx<'_>, node: AstKind<'_>) -> bool {
    match node {
        AstKind::AssignmentExpression(a) => match &a.left {
            AssignmentTarget::AssignmentTargetIdentifier(id) => {
                ctx.semantic.is_reference_to_global_variable(id) && protected("", id.name.as_str())
            }
            target => target
                .as_simple_assignment_target()
                .and_then(|t| t.as_member_expression())
                .is_some_and(|m| protected_member(ctx, m)),
        },
        AstKind::UnaryExpression(u) if u.operator.is_delete() => u
            .argument
            .get_member_expr()
            .is_some_and(|m| protected_member(ctx, m)),
        _ => false,
    }
}

fn defines(ctx: &TsCtx<'_>, initial: &Expression<'_>, owner: &str) -> bool {
    let mut pending = vec![initial];
    let mut seen = HashSet::new();
    while let Some(expr) = pending.pop() {
        let expr = resolve(ctx, expr);
        if !seen.insert(expr.span()) {
            continue;
        }
        match expr {
            Expression::ObjectExpression(obj) => {
                for p in &obj.properties {
                    match p {
                        ObjectPropertyKind::SpreadProperty(s) => pending.push(&s.argument),
                        ObjectPropertyKind::ObjectProperty(p)
                            if key(ctx, &p.key, p.computed)
                                .is_some_and(|k| protected(owner, k)) =>
                        {
                            return true;
                        }
                        ObjectPropertyKind::ObjectProperty(_) => (),
                    }
                }
            }
            Expression::LogicalExpression(l) => pending.extend([&l.left, &l.right]),
            Expression::ConditionalExpression(c) => pending.extend([&c.consequent, &c.alternate]),
            Expression::SequenceExpression(s) => pending.extend(s.expressions.last()),
            _ => (),
        }
    }
    false
}

pub(super) struct Invocation<'a> {
    pub(super) callee: &'a Expression<'a>,
    pub(super) path: Option<Vec<&'a str>>,
    native_path: Option<String>,
    pub(super) args: Vec<Option<&'a Expression<'a>>>,
}
impl<'a> Invocation<'a> {
    pub(super) fn arg(&self, i: usize) -> Option<&'a Expression<'a>> {
        self.args.get(i).copied().flatten()
    }
}
fn array_args<'a>(
    ctx: &TsCtx<'a>,
    arg: Option<&'a Expression<'a>>,
) -> Vec<Option<&'a Expression<'a>>> {
    if let Some(Expression::ArrayExpression(a)) = arg.map(|a| resolve(ctx, a)) {
        a.elements.iter().map(|e| e.as_expression()).collect()
    } else {
        Vec::new()
    }
}
pub(super) fn invoked<'a>(ctx: &TsCtx<'a>, call: &'a CallExpression<'a>) -> Invocation<'a> {
    let mut callee = resolve(ctx, &call.callee);
    let mut args: Vec<_> = call.arguments.iter().map(Argument::as_expression).collect();
    let mut seen = HashSet::new();
    let (path, native_path) = loop {
        let parts = path(ctx, callee);
        let native_path = parts.as_deref().map(native_name);
        if !seen.insert(callee.span()) {
            break (parts, native_path);
        }
        if let Expression::CallExpression(c) = callee
            && let Some(m) = resolve(ctx, &c.callee).get_member_expr()
            && member_key(ctx, m) == Some("bind")
        {
            args.splice(..0, c.arguments.iter().skip(1).map(Argument::as_expression));
            callee = resolve(ctx, m.object());
            continue;
        }
        if native_path.as_deref() == Some("Reflect.apply")
            && let Some(target) = args.first().copied().flatten()
        {
            args = array_args(ctx, args.get(2).copied().flatten());
            callee = resolve(ctx, target);
            continue;
        }
        let Some(m) = callee.get_member_expr() else {
            break (parts, native_path);
        };
        match member_key(ctx, m) {
            Some("call") => {
                args.drain(..args.len().min(1));
            }
            Some("apply") => args = array_args(ctx, args.get(1).copied().flatten()),
            _ => break (parts, native_path),
        }
        callee = resolve(ctx, m.object());
    };
    Invocation {
        callee,
        path,
        native_path,
        args,
    }
}

pub(super) fn call_writes(ctx: &TsCtx<'_>, call: &Invocation<'_>) -> bool {
    let Some(owner) = call.arg(0).and_then(|e| native(ctx, e)) else {
        return false;
    };
    match call.native_path.as_deref() {
        Some(
            "Object.defineProperty"
            | "Reflect.defineProperty"
            | "Reflect.set"
            | "Reflect.deleteProperty",
        ) => call
            .arg(1)
            .and_then(|e| literal(ctx, e))
            .is_some_and(|k| protected(&owner, k)),
        Some("Object.defineProperties") => call.arg(1).is_some_and(|e| defines(ctx, e, &owner)),
        Some("Object.assign") => call
            .args
            .iter()
            .skip(1)
            .flatten()
            .any(|e| defines(ctx, e, &owner)),
        _ => false,
    }
}

fn method<'a>(ctx: &TsCtx<'a>, callee: &'a Expression<'a>) -> Option<&'a str> {
    if let Some(m) = callee.get_member_expr() {
        return member_key(ctx, m);
    }
    let Expression::Identifier(id) = callee else {
        return None;
    };
    let AstKind::VariableDeclarator(v) = declaration(ctx, id)? else {
        return None;
    };
    let BindingPattern::ObjectPattern(pattern) = &v.id else {
        return None;
    };
    super::values::destructured(ctx, pattern, |b| b.name == id.name)
}
fn observation(ctx: &TsCtx<'_>, call: &Invocation<'_>) -> bool {
    if call.args.len() != 2 {
        return false;
    }
    let Some(Expression::ObjectExpression(obj)) = call.arg(1) else {
        return false;
    };
    let [ObjectPropertyKind::ObjectProperty(p)] = obj.properties.as_slice() else {
        return false;
    };
    !p.computed
        && !p.method
        && key(ctx, &p.key, false) == Some("spy")
        && matches!(&p.value,Expression::BooleanLiteral(v) if v.value)
}

pub(super) fn framework_call(ctx: &TsCtx<'_>, call: &Invocation<'_>) -> bool {
    if method(ctx, call.callee).is_some_and(|s| {
        matches!(
            s,
            "mockImplementation"
                | "mockImplementationOnce"
                | "mockReturnValue"
                | "mockReturnValueOnce"
                | "mockResolvedValue"
                | "mockResolvedValueOnce"
                | "mockRejectedValue"
                | "mockRejectedValueOnce"
                | "mockReturnThis"
                | "withImplementation"
                | "mockStepResult"
                | "mockStepError"
        )
    }) {
        return true;
    }
    let Some(parts) = &call.path else {
        return false;
    };

    match parts.as_slice() {
        [
            "@tauri-apps/api/mocks",
            "mockIPC" | "mockWindows" | "mockConvertFileSrc",
        ] => true,
        [owner, m] | ["vitest" | "@jest/globals", owner, m]
            if matches!(*owner, "vi" | "vitest" | "jest") =>
        {
            match *m {
                "fn" => call
                    .arg(0)
                    .is_some_and(|e| !super::values::resolves_to_global(ctx, e, "undefined")),
                "mockObject" => *owner == "jest" || !observation(ctx, call),
                "mock"
                | "doMock"
                | "stubGlobal"
                | "unstable_mockModule"
                | "importMock"
                | "createMockFromModule"
                | "genMockFromModule"
                | "setMock"
                | "replaceProperty"
                | "enableAutomock" => true,
                _ => false,
            }
        }
        _ => false,
    }
}
