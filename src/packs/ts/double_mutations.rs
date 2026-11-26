use super::{
    native_properties::{owner_properties, protected_global},
    values::{MemberKeys, key, literal, path, resolve},
};
use crate::engine::TsCtx;
use oxc::ast::{
    AstKind,
    ast::{
        AssignmentTarget, AssignmentTargetMaybeDefault, AssignmentTargetProperty, Expression,
        IdentifierReference, MemberExpression,
    },
};

fn native<'a>(ctx: &TsCtx<'a>, expr: &'a Expression<'a>) -> Option<fn(&str) -> bool> {
    owner_properties(&path(ctx, expr)?)
}
fn protected_member<'a>(ctx: &TsCtx<'a>, m: &'a MemberExpression<'a>) -> bool {
    native(ctx, m.object()).is_some_and(|protected| MemberKeys(ctx).read(m).is_none_or(protected))
}

fn protected_identifier(ctx: &TsCtx<'_>, id: &IdentifierReference<'_>) -> bool {
    ctx.semantic.is_reference_to_global_variable(id) && protected_global(id.name.as_str())
}

fn assignment_writes<'a>(ctx: &TsCtx<'a>, root: &'a AssignmentTarget<'a>) -> bool {
    let target = |candidate: &'a AssignmentTargetMaybeDefault<'a>| match candidate {
        AssignmentTargetMaybeDefault::AssignmentTargetWithDefault(default) => {
            Some(&default.binding)
        }
        other => other.as_assignment_target(),
    };
    let mut pending = vec![root];
    while let Some(next) = pending.pop() {
        match next {
            AssignmentTarget::AssignmentTargetIdentifier(id) => {
                if protected_identifier(ctx, id) {
                    return true;
                }
            }
            AssignmentTarget::ArrayAssignmentTarget(array) => {
                pending.extend(array.elements.iter().flatten().filter_map(target));
                pending.extend(array.rest.iter().map(|rest| &rest.target));
            }
            AssignmentTarget::ObjectAssignmentTarget(object) => {
                pending.extend(object.rest.iter().map(|rest| &rest.target));
                for property in &object.properties {
                    match property {
                        AssignmentTargetProperty::AssignmentTargetPropertyProperty(p) => {
                            pending.extend(target(&p.binding));
                        }
                        AssignmentTargetProperty::AssignmentTargetPropertyIdentifier(p) => {
                            if protected_identifier(ctx, &p.binding) {
                                return true;
                            }
                        }
                    }
                }
            }
            target => {
                if target
                    .as_member_expression()
                    .is_some_and(|m| protected_member(ctx, m))
                {
                    return true;
                }
            }
        }
    }
    false
}

pub(super) fn writes<'a>(ctx: &TsCtx<'a>, node: AstKind<'a>) -> bool {
    match node {
        AstKind::AssignmentExpression(a) => assignment_writes(ctx, &a.left),
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
        match self.path.as_deref()? {
            ["Object", "assign"] | ["Object.assign"] => Some(Mutation::Assign),
            ["Object", "defineProperties"] | ["Object.defineProperties"] => {
                Some(Mutation::DefineMany)
            }
            ["Object" | "Reflect", "defineProperty"]
            | ["Reflect", "set"]
            | ["Object.defineProperty" | "Reflect.defineProperty" | "Reflect.set"] => {
                Some(Mutation::SetProperty)
            }
            ["Reflect", "deleteProperty"] | ["Reflect.deleteProperty"] => Some(Mutation::Delete),
            _ => None,
        }
    }
}
pub(super) fn call_writes<'a>(ctx: &TsCtx<'a>, call: &Invocation<'a>) -> bool {
    let Some(kind) = call.mutation() else {
        return false;
    };
    let Some(protected) = call.arg(0).and_then(|e| native(ctx, e)) else {
        return false;
    };
    let defines =
        |e| super::values::any_property(ctx, e, |p| key(ctx, &p.key).is_none_or(protected));
    match kind {
        Mutation::SetProperty | Mutation::Delete => call.arg(1).is_some_and(|e| {
            literal(ctx, e).map_or_else(|| !resolve(ctx, e).is_literal(), protected)
        }),
        Mutation::DefineMany => call.arg(1).is_some_and(defines),
        Mutation::Assign => ctx
            .counted(call.args.iter().skip(1))
            .flatten()
            .any(|e| defines(e)),
    }
}

fn observation<'a>(ctx: &TsCtx<'a>, call: &Invocation<'a>) -> bool {
    if call.unreadable_args.is_some() || call.args.len() != 2 {
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

pub(super) fn banned_mock_method(method: &str) -> bool {
    matches!(
        method,
        "stubGlobal"
            | "unstable_mockModule"
            | "importMock"
            | "createMockFromModule"
            | "genMockFromModule"
            | "setMock"
            | "replaceProperty"
            | "enableAutomock"
    )
}

fn local_class<'a>(
    ctx: &TsCtx<'a>,
    receiver: &'a Expression<'a>,
) -> Option<(&'a oxc::ast::ast::Class<'a>, bool)> {
    match resolve(ctx, receiver) {
        Expression::NewExpression(new) => {
            let id = super::values::binding_value(ctx, &new.callee).get_identifier_reference()?;
            match super::values::declaration(ctx, id)? {
                AstKind::Class(class) => Some((class, false)),
                _ => None,
            }
        }
        Expression::ThisExpression(this) => {
            let function = super::values::enclosing_functions(ctx, this.node_id.get())
                .find(|node| matches!(node.kind(), AstKind::Function(_)))?;
            let nodes = ctx.semantic.nodes();
            let AstKind::MethodDefinition(method) = nodes.parent_kind(function.id()) else {
                return None;
            };
            ctx.ancestors(method.node_id.get())
                .find_map(|node| match node.kind() {
                    AstKind::Class(class) => Some((class, method.r#static)),
                    _ => None,
                })
        }
        _ => None,
    }
}

pub(super) fn replacement_method<'a>(
    ctx: &TsCtx<'a>,
    receiver: &'a Expression<'a>,
    method: &str,
) -> bool {
    matches!(
        method,
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
    ) && !local_class(ctx, receiver).is_some_and(|(class, is_static)| {
        ctx.counted(class.body.body.iter()).any(|element| {
            matches!(element, oxc::ast::ast::ClassElement::MethodDefinition(m)
                if m.r#static == is_static && super::values::static_name(ctx, &m.key).is_some_and(|name| name == method))
        })
    })
}

pub(super) fn framework_call<'a>(ctx: &TsCtx<'a>, call: &Invocation<'a>) -> bool {
    if super::values::receiver(ctx, call.callee)
        .is_some_and(|(owner, method)| replacement_method(ctx, owner, method))
    {
        return true;
    }
    if let Some(parts) = &call.path {
        let origin = super::values::origin_path(ctx, call.callee).map(|(origin, _)| origin);
        return framework_path(ctx, call, origin, parts);
    }
    super::values::any_assigned_path(ctx, call.callee, |origin, path| {
        framework_path(ctx, call, Some(origin), path)
    }) || super::values::receiver(ctx, call.callee).is_some_and(|(owner, method)| {
        super::values::any_assigned_path(ctx, owner, |origin, path| {
            let mut parts = path.to_vec();
            parts.push(method);
            framework_path(ctx, call, Some(origin), &parts)
        })
    })
}

fn framework_path<'a>(
    ctx: &TsCtx<'a>,
    call: &Invocation<'a>,
    origin: Option<super::values::Origin>,
    parts: &[&str],
) -> bool {
    if parts.len() == 2
        && matches!(parts[0], "vi" | "vitest" | "jest")
        && origin != Some(super::values::Origin::Global)
    {
        return false;
    }
    match parts {
        [
            "@tauri-apps/api/mocks",
            "mockIPC" | "mockWindows" | "mockConvertFileSrc",
        ] => true,
        [owner, m] | ["vitest" | "@jest/globals", owner, m]
            if matches!(*owner, "vi" | "vitest" | "jest") =>
        {
            match *m {
                "fn" => {
                    call.template_arg
                        || call.unreadable_args.is_some()
                        || call.arg(0).is_some_and(|e| {
                            !super::values::resolves_to_global(ctx, e, "undefined")
                        })
                }
                "mockObject" | "mock" | "doMock" => *owner == "jest" || !observation(ctx, call),
                m => banned_mock_method(m),
            }
        }
        _ => false,
    }
}
