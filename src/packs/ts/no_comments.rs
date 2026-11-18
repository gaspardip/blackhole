use crate::{
    engine::{Family, Findings, Rule, Run, TsCtx},
    packs::core::no_suppression::suppressed_comment,
};
use oxc::{
    ast::{
        AstKind,
        ast::{
            Comment, ExportDefaultDeclarationKind, IdentifierReference, ModuleExportName,
            TSAccessibility, TSTypeName,
        },
    },
    span::GetSpan,
    syntax::node::NodeId,
};
use std::collections::{HashMap, HashSet};

pub(crate) const RULE: Rule = Rule {
    id: "ts/no-comments",
    family: Family::Narration,
    provenance: "Port of landmines style/no-comments.",
    fix: "Delete the comment. Put its meaning in names and its rationale in the pull request.",
    run: Run::Ts(check),
};

fn word_end(rest: &str) -> bool {
    !rest.starts_with(|c: char| c.is_ascii_alphanumeric() || c == '_')
}

fn directive(raw: &str) -> bool {
    let value = raw
        .strip_prefix('!')
        .unwrap_or(raw)
        .split(['\n', '\r', '\u{2028}', '\u{2029}'])
        .map(|line| {
            line.trim_start().strip_prefix('*').map_or(line, |rest| {
                rest.strip_prefix(char::is_whitespace).unwrap_or(rest)
            })
        })
        .collect::<Vec<_>>()
        .join("\n");
    let value = value.trim();
    let first = value.split_whitespace().next().unwrap_or("");
    matches!(
        first,
        "eslint-enable"
            | "eslint-enable-line"
            | "eslint-enable-next-line"
            | "oxlint-enable"
            | "oxlint-enable-line"
            | "oxlint-enable-next-line"
            | "@ts-check"
            | "prettier-ignore"
            | "prettier-ignore-start"
            | "prettier-ignore-end"
            | "oxfmt-ignore"
            | "oxfmt-ignore-start"
            | "oxfmt-ignore-end"
    ) || value.strip_prefix(['@', '#']).is_some_and(|v| {
        matches!(
            v,
            "__PURE__" | "__NO_SIDE_EFFECTS__" | "__INLINE__" | "__NOINLINE__"
        )
    }) || value == "@vite-ignore"
        || value.split_once(':').is_some_and(|(v, _)| {
            matches!(
                v,
                "webpackChunkName"
                    | "webpackMode"
                    | "webpackPrefetch"
                    | "webpackPreload"
                    | "webpackIgnore"
                    | "webpackInclude"
                    | "webpackExclude"
                    | "webpackExports"
                    | "webpackFetchPriority"
            )
        })
        || (matches!(
            first,
            "@jsx" | "@jsxFrag" | "@jsxImportSource" | "@jsxRuntime"
        ) && value.split_whitespace().nth(1).is_some())
        || value.strip_prefix('/').is_some_and(|v| {
            matches!(
                v.split_whitespace().next(),
                Some("<reference" | "<amd-module" | "<amd-dependency")
            ) && v.trim_start().contains(char::is_whitespace)
        })
        || value.strip_prefix('#').is_some_and(|v| {
            v.trim_start().starts_with("sourceURL=")
                || v.trim_start().starts_with("sourceMappingURL=")
        })
        || value.lines().any(|v| {
            v.starts_with("SPDX-License-Identifier:")
                || v.strip_prefix("@license").is_some_and(word_end)
        })
}

fn hidden(kind: AstKind<'_>) -> bool {
    let (access, key) = match kind {
        AstKind::MethodDefinition(n) => (n.accessibility, &n.key),
        AstKind::PropertyDefinition(n) => (n.accessibility, &n.key),
        AstKind::AccessorProperty(n) => (n.accessibility, &n.key),
        AstKind::FunctionBody(_) | AstKind::BlockStatement(_) | AstKind::StaticBlock(_) => {
            return true;
        }
        _ => return false,
    };
    matches!(
        access,
        Some(TSAccessibility::Private | TSAccessibility::Protected)
    ) || key.is_private_identifier()
}

fn member(kind: AstKind<'_>) -> bool {
    matches!(
        kind,
        AstKind::ObjectProperty(_)
            | AstKind::TSInterfaceDeclaration(_)
            | AstKind::TSTypeAliasDeclaration(_)
            | AstKind::TSPropertySignature(_)
            | AstKind::TSMethodSignature(_)
            | AstKind::TSCallSignatureDeclaration(_)
            | AstKind::TSConstructSignatureDeclaration(_)
            | AstKind::TSIndexSignature(_)
            | AstKind::TSEnumMember(_)
            | AstKind::MethodDefinition(_)
            | AstKind::PropertyDefinition(_)
    )
}

fn local_export(
    ctx: &TsCtx<'_>,
    ident: &IdentifierReference<'_>,
    declarations: &mut HashSet<NodeId>,
) {
    let scoping = ctx.semantic.scoping();
    let nodes = ctx.semantic.nodes();
    if let Some(symbol) = scoping.get_root_binding(ident.name) {
        for id in scoping.symbol_declarations(symbol) {
            if let Some(top) = std::iter::once(id)
                .chain(nodes.ancestor_ids(id))
                .find(|n| matches!(nodes.parent_kind(*n), AstKind::Program(_)))
            {
                declarations.insert(top);
            }
        }
    }
}

fn public_declarations(ctx: &TsCtx<'_>) -> HashSet<NodeId> {
    let nodes = ctx.semantic.nodes();
    let ambient = ctx.program.source_type.is_typescript_definition()
        && !ctx
            .program
            .body
            .iter()
            .any(oxc::ast::ast::Statement::is_module_declaration);
    let mut declarations = HashSet::new();
    for (id, node) in nodes
        .iter_enumerated()
        .filter(|(id, _)| matches!(nodes.parent_kind(*id), AstKind::Program(_)))
    {
        let kind = node.kind();
        match kind {
            AstKind::ExportNamedDeclaration(n) => {
                for spec in &n.specifiers {
                    if let ModuleExportName::IdentifierReference(r) = &spec.local {
                        local_export(ctx, r, &mut declarations);
                    }
                }
            }
            AstKind::ExportDefaultDeclaration(n) => {
                if let ExportDefaultDeclarationKind::Identifier(r) = &n.declaration {
                    local_export(ctx, r, &mut declarations);
                }
            }
            _ => {}
        }
        let public = matches!(
            kind,
            AstKind::ExportNamedDeclaration(_)
                | AstKind::ExportDefaultDeclaration(_)
                | AstKind::ExportAllDeclaration(_)
                | AstKind::ExportDeclaration(_)
                | AstKind::ExportFromDeclaration(_)
        ) || matches!(kind,AstKind::TSGlobalDeclaration(n) if n.declare)
            || matches!(kind,AstKind::TSNamespaceDeclaration(n) if n.declare)
            || matches!(kind,AstKind::TSExternalModuleDeclaration(n) if n.declare);
        if public || (ambient && !matches!(kind, AstKind::Program(_))) {
            declarations.insert(id);
        }
    }
    declarations
}

struct Exposure {
    owners: HashMap<NodeId, NodeId>,
    public: HashSet<NodeId>,
}

fn exposure(ctx: &TsCtx<'_>) -> Exposure {
    let nodes = ctx.semantic.nodes();
    let mut public = public_declarations(ctx);
    let mut owners = HashMap::new();
    let mut edges: HashMap<NodeId, Vec<NodeId>> = HashMap::new();
    // Semantic nodes are in preorder, so a node inherits its parent's enclosing declaration.
    for (id, node) in nodes.iter_enumerated() {
        let kind = node.kind();
        let parent = nodes.parent_id(id);
        let inherited = owners.get(&parent).copied().filter(|_| !hidden(kind) && !matches!(nodes.kind(parent),AstKind::ArrowFunctionExpression(n) if n.body.node_id() == id));
        let owner = if public.contains(&id)
            || matches!(
                kind,
                AstKind::TSInterfaceDeclaration(_) | AstKind::TSTypeAliasDeclaration(_)
            ) {
            if let Some(parent_owner) = inherited {
                edges.entry(parent_owner).or_default().push(id);
            }
            id
        } else if let Some(owner) = inherited {
            owner
        } else {
            continue;
        };
        owners.insert(id, owner);
        if let AstKind::TSTypeReference(r) = kind
            && let TSTypeName::IdentifierReference(name) = &r.type_name
            && let Some(symbol) = name
                .reference_id
                .get()
                .and_then(|id| ctx.semantic.scoping().get_reference(id).symbol_id())
        {
            edges.entry(owner).or_default().extend(
                ctx.semantic
                    .scoping()
                    .symbol_declarations(symbol)
                    .filter(|id| {
                        matches!(
                            nodes.kind(*id),
                            AstKind::TSInterfaceDeclaration(_) | AstKind::TSTypeAliasDeclaration(_)
                        )
                    }),
            );
        }
    }
    let mut pending: Vec<_> = public.iter().copied().collect();
    while let Some(owner) = pending.pop() {
        if let Some(targets) = edges.remove(&owner) {
            pending.extend(targets.into_iter().filter(|id| public.insert(*id)));
        }
    }
    Exposure { owners, public }
}

fn doc_before(ctx: &TsCtx<'_>, mut start: u32, directives: &HashSet<u32>) -> Option<u32> {
    let end = ctx.comments.partition_point(|c| c.span.end <= start);
    for c in ctx.comments[..end].iter().rev() {
        if !ctx.text[c.span.end as usize..start as usize]
            .trim()
            .is_empty()
        {
            return None;
        }
        if !directives.contains(&c.span.start) {
            let raw = c.content_span().source_text(ctx.text);
            return (c.is_block()
                && raw.starts_with('*')
                && !["@internal", "@private"].iter().any(|tag| {
                    raw.match_indices(tag)
                        .any(|(i, _)| word_end(&raw[i + tag.len()..]))
                }))
            .then_some(c.span.start);
        }
        start = c.span.start;
    }
    None
}

fn warning(comment: &Comment, raw: &str) -> bool {
    comment.is_line()
        && raw
            .trim_start()
            .strip_prefix("WARNING:")
            .is_some_and(|v| !v.trim().is_empty())
}

fn check(ctx: &TsCtx<'_>, findings: &mut Findings) {
    if ctx.comments.is_empty() {
        return;
    }
    let suppressed: HashSet<_> = ctx
        .comments
        .iter()
        .filter(|c| suppressed_comment(c.span.source_text(ctx.text)))
        .map(|c| c.span.start)
        .collect();
    let directives: HashSet<_> = ctx
        .comments
        .iter()
        .filter(|c| {
            directive(c.content_span().source_text(ctx.text)) && !suppressed.contains(&c.span.start)
        })
        .map(|c| c.span.start)
        .collect();
    let exposure = exposure(ctx);
    let documented: HashSet<_> = ctx
        .semantic
        .nodes()
        .iter_enumerated()
        .filter(|(id, n)| {
            exposure.public.contains(id)
                || (member(n.kind())
                    && exposure
                        .owners
                        .get(id)
                        .is_some_and(|owner| exposure.public.contains(owner)))
                || matches!(
                    ctx.semantic.nodes().parent_kind(*id),
                    AstKind::ExportDeclaration(_) | AstKind::ExportDefaultDeclaration(_)
                ) && !matches!(n.kind(), AstKind::IdentifierReference(_))
        })
        .filter_map(|(_, n)| doc_before(ctx, n.span().start, &directives))
        .collect();
    for c in ctx.comments {
        if suppressed.contains(&c.span.start)
            || !directives.contains(&c.span.start)
                && !documented.contains(&c.span.start)
                && !warning(c, c.content_span().source_text(ctx.text))
        {
            findings.report(c.span.start as usize, None);
        }
    }
}
