use crate::engine::{Family, Findings, Gate, Rule, Run, TsCtx};
use oxc::{ast::AstKind, span::GetSpan};

pub(crate) const RULE: Rule = Rule {
    id: "ts/no-non-null-assertion",
    family: Family::Types,
    gate: Gate::Blocking,
    provenance: "Agents add ! to silence a nullability error instead of handling the missing value.",
    fix: "Handle the missing value where it can be missing, or change the type so it cannot be.",
    run: Run::Ts(check),
};

fn check(ctx: &TsCtx<'_>, findings: &mut Findings) {
    for node in ctx.nodes() {
        let start = match node.kind() {
            AstKind::TSNonNullExpression(n) => {
                findings.report(n.span.end as usize - 1, None);
                continue;
            }
            AstKind::VariableDeclarator(n) if n.definite => n.id.span().end,
            AstKind::PropertyDefinition(n) if n.definite => n.key.span().end,
            AstKind::AccessorProperty(n) if n.definite => n.key.span().end,
            _ => continue,
        };
        let mut comments = ctx.comments.partition_point(|c| c.span.end <= start);
        for (i, byte) in ctx
            .text
            .as_bytes()
            .iter()
            .enumerate()
            .take(node.span().end as usize)
            .skip(start as usize)
        {
            if let Some(comment) = ctx.comments.get(comments) {
                if i >= comment.span.end as usize {
                    comments += 1;
                }
                if ctx
                    .comments
                    .get(comments)
                    .is_some_and(|c| c.span.start as usize <= i)
                {
                    continue;
                }
            }
            if *byte == b'!' {
                findings.report(i, None);
                break;
            }
        }
    }
}
