use super::{
    native_properties::protected,
    values::{MemberKeys, key, literal, path, resolve},
};
use crate::engine::TsCtx;
use oxc::ast::{
    AstKind,
    ast::{AssignmentTarget, Expression, MemberExpression},
};

fn native(ctx: &TsCtx<'_>, expr: &Expression<'_>) -> Option<String> {
    let parts = path(ctx, expr)?;
    Some(native_name(&parts))
}
fn native_name(parts: &[&str]) -> String {
    let skip = usize::from(
        parts
            .first()
            .is_some_and(|s| super::values::global_object(s)),
    );
    parts[skip..].join(".")
}

fn protected_member(ctx: &TsCtx<'_>, m: &MemberExpression<'_>) -> bool {
    native(ctx, m.object())
        .and_then(|owner| super::native_properties::owner_properties(&owner))
        .is_some_and(|protected| MemberKeys::Resolve(ctx).read(m).is_none_or(protected))
}

pub(super) fn writes(ctx: &TsCtx<'_>, node: AstKind<'_>) -> bool {
    match node {
        AstKind::AssignmentExpression(a) => match &a.left {
            AssignmentTarget::AssignmentTargetIdentifier(id) => {
                ctx.semantic.is_reference_to_global_variable(id) && protected("", id.name.as_str())
            }
            target => target
                .as_member_expression()
                .is_some_and(|m| protected_member(ctx, m)),
        },
        AstKind::UnaryExpression(u) if u.operator.is_delete() => u
            .argument
            .get_member_expr()
            .is_some_and(|m| protected_member(ctx, m)),
        _ => false,
    }
}

pub(super) enum Mutation {
    Assign,
    DefineMany,
    SetProperty,
    Delete,
}

use super::values::Invocation;
impl Invocation<'_> {
    pub(super) fn mutation(&self) -> Option<Mutation> {
        match native_name(self.path.as_deref()?).as_str() {
            "Object.assign" => Some(Mutation::Assign),
            "Object.defineProperties" => Some(Mutation::DefineMany),
            "Object.defineProperty" | "Reflect.defineProperty" | "Reflect.set" => {
                Some(Mutation::SetProperty)
            }
            "Reflect.deleteProperty" => Some(Mutation::Delete),
            _ => None,
        }
    }
}
pub(super) fn call_writes(ctx: &TsCtx<'_>, call: &Invocation<'_>) -> bool {
    let Some(kind) = call.mutation() else {
        return false;
    };
    let Some(protected) = call
        .arg(0)
        .and_then(|e| native(ctx, e))
        .and_then(|owner| super::native_properties::owner_properties(&owner))
    else {
        return false;
    };
    let defines =
        |e| super::values::any_property(ctx, e, |p| key(ctx, &p.key).is_none_or(protected));
    match kind {
        Mutation::SetProperty | Mutation::Delete => call.arg(1).is_some_and(|e| {
            literal(ctx, e).map_or_else(|| !resolve(ctx, e).is_literal(), protected)
        }),
        Mutation::DefineMany => call.arg(1).is_some_and(defines),
        Mutation::Assign => call.args.iter().skip(1).flatten().any(|e| defines(e)),
    }
}

fn observation(ctx: &TsCtx<'_>, call: &Invocation<'_>) -> bool {
    if call.args.len() != 2 {
        return false;
    }
    let Some(opts) = call.arg(1).and_then(|e| super::values::properties(ctx, e)) else {
        return false;
    };
    !opts.unknown
        && opts.named.len() == 1
        && opts.named.get("spy").is_some_and(|p| {
            !p.method && matches!(resolve(ctx,&p.value),Expression::BooleanLiteral(v) if v.value)
        })
}

pub(super) fn framework_call(ctx: &TsCtx<'_>, call: &Invocation<'_>) -> bool {
    if super::values::receiver(ctx, call.callee).is_some_and(|(_, s)| {
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

    match super::values::unqualified(parts) {
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
                "mockObject" | "mock" | "doMock" => *owner == "jest" || !observation(ctx, call),
                "stubGlobal"
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
