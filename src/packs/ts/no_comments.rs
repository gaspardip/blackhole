use crate::{
    engine::{Family, Findings, Gate, Rule, Run, TsCtx},
    packs::core::no_suppression::suppressed_comment,
};
use oxc::{
    ast::{
        AstKind,
        ast::{Comment, Expression, ModuleExportName, TSAccessibility, TSTypeName},
    },
    span::GetSpan,
    syntax::{line_terminator::is_line_terminator, node::NodeId},
};
use std::collections::{HashMap, HashSet};

pub(crate) const RULE: Rule = Rule {
    id: "ts/no-comments",
    family: Family::Narration,
    gate: Gate::Blocking,
    provenance: "Port of landmines style/no-comments.",
    fix: "Delete the comment. Put its meaning in names and its rationale in the pull request.",
    run: Run::Ts(check),
};

fn word_end(rest: &str) -> bool {
    !rest.starts_with(|c: char| c.is_ascii_alphanumeric() || c == '_')
}

fn js_whitespace(c: char) -> bool {
    oxc::syntax::identifier::is_white_space(c) || is_line_terminator(c)
}

fn directive(ctx: &TsCtx<'_>, raw: &str) -> bool {
    ctx.record_string_work(raw.len());
    let value = raw
        .strip_prefix('!')
        .unwrap_or(raw)
        .split(is_line_terminator)
        .map(|line| {
            line.trim_start_matches(js_whitespace)
                .strip_prefix('*')
                .map_or(line, |rest| {
                    rest.strip_prefix(js_whitespace).unwrap_or(rest)
                })
        })
        .collect::<Vec<_>>()
        .join("\n");
    let value = value.trim_matches(js_whitespace);
    let (first, argument) = value.split_once(js_whitespace).unwrap_or((value, ""));
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
        ) && !argument.is_empty())
        || value.strip_prefix('/').is_some_and(|v| {
            v.trim_start_matches(js_whitespace)
                .split_once(js_whitespace)
                .is_some_and(|(tag, _)| {
                    matches!(tag, "<reference" | "<amd-module" | "<amd-dependency")
                })
        })
        || value.strip_prefix('#').is_some_and(|v| {
            let v = v.trim_start_matches(js_whitespace);
            v.starts_with("sourceURL=") || v.starts_with("sourceMappingURL=")
        })
        || value.split('\n').any(|v| {
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

fn public_declarations(ctx: &TsCtx<'_>) -> HashSet<NodeId> {
    let nodes = ctx.semantic.nodes();
    let ambient = ctx.program.source_type.is_typescript_definition()
        && !ctx
            .program
            .body
            .iter()
            .any(oxc::ast::ast::Statement::is_module_declaration);
    let mut declarations = HashSet::new();
    let scoping = ctx.semantic.scoping();
    let mut bindings = HashSet::new();
    for (id, node) in ctx
        .nodes_with_ids()
        .filter(|(id, _)| matches!(nodes.parent_kind(*id), AstKind::Program(_)))
    {
        let kind = node.kind();
        match kind {
            AstKind::ExportNamedDeclaration(n) => {
                for spec in &n.specifiers {
                    ctx.record_work(1);
                    if let ModuleExportName::IdentifierReference(r) = &spec.local {
                        ctx.record_string_work(r.name.len());
                        bindings.extend(scoping.get_root_binding(r.name));
                    }
                }
            }
            AstKind::ExportDefaultDeclaration(n) => {
                if let Some(Expression::Identifier(r)) = n
                    .declaration
                    .as_expression()
                    .map(Expression::without_parentheses)
                {
                    ctx.record_string_work(r.name.len());
                    bindings.extend(scoping.get_root_binding(r.name));
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
    if bindings.is_empty() {
        return declarations;
    }
    // Semantic nodes are in preorder, so a node inherits its parent's top-level statement.
    let mut tops: Vec<NodeId> = Vec::with_capacity(nodes.len());
    for (id, _) in ctx.nodes_with_ids() {
        let parent = nodes.parent_id(id);
        let top = match nodes.kind(parent) {
            AstKind::Program(_) => id,
            _ => tops[parent.index()],
        };
        tops.push(top);
    }
    for symbol in bindings {
        for id in ctx.counted(scoping.symbol_declarations(symbol)) {
            declarations.insert(tops[id.index()]);
        }
    }
    declarations
}

struct Exposure {
    owners: Vec<Option<NodeId>>,
    public: HashSet<NodeId>,
}

impl Exposure {
    fn can_document(&self, id: NodeId, kind: AstKind<'_>) -> bool {
        self.public.contains(&id)
            || member(kind)
                && self
                    .owners
                    .get(id.index())
                    .copied()
                    .flatten()
                    .is_some_and(|owner| self.public.contains(&owner))
    }
}

fn type_declaration(kind: AstKind<'_>) -> bool {
    matches!(
        kind,
        AstKind::TSInterfaceDeclaration(_) | AstKind::TSTypeAliasDeclaration(_)
    )
}

fn exposure(ctx: &TsCtx<'_>) -> Exposure {
    let nodes = ctx.semantic.nodes();
    let mut public = public_declarations(ctx);
    if public.is_empty() {
        return Exposure {
            owners: Vec::new(),
            public,
        };
    }
    let mut owners = vec![None; nodes.len()];
    let mut references: HashMap<NodeId, HashSet<_>> = HashMap::new();
    let mut edges: HashMap<NodeId, Vec<NodeId>> = HashMap::new();
    // Semantic nodes are in preorder, so a node inherits its parent's enclosing declaration.
    for (id, node) in ctx.nodes_with_ids() {
        let kind = node.kind();
        let parent = nodes.parent_id(id);
        let inherited = owners[parent.index()].filter(|_| !hidden(kind) && !matches!(nodes.kind(parent),AstKind::ArrowFunctionExpression(n) if n.body.node_id() == id));
        let owner = if public.contains(&id) || type_declaration(kind) {
            if let Some(parent_owner) = inherited {
                edges.entry(parent_owner).or_default().push(id);
            }
            id
        } else if let Some(owner) = inherited {
            owner
        } else {
            continue;
        };
        owners[id.index()] = Some(owner);
        if let AstKind::TSTypeReference(r) = kind
            && let TSTypeName::IdentifierReference(name) = &r.type_name
            && let Some(symbol) = name
                .reference_id
                .get()
                .and_then(|id| ctx.semantic.scoping().get_reference(id).symbol_id())
        {
            references.entry(owner).or_default().insert(symbol);
        }
    }
    let scoping = ctx.semantic.scoping();
    let mut expanded = HashSet::new();
    let mut pending: Vec<_> = public.iter().copied().collect();
    while let Some(owner) = pending.pop() {
        ctx.record_work(1);
        for symbol in ctx
            .counted(references.remove(&owner).into_iter().flatten())
            .filter(|s| expanded.insert(*s))
        {
            pending.extend(
                ctx.counted(scoping.symbol_declarations(symbol))
                    .filter(|id| type_declaration(nodes.kind(*id)) && public.insert(*id)),
            );
        }
        if let Some(targets) = edges.remove(&owner) {
            pending.extend(
                ctx.counted(targets.into_iter())
                    .filter(|id| public.insert(*id)),
            );
        }
    }
    Exposure { owners, public }
}

fn doc_before(ctx: &TsCtx<'_>, mut start: u32, directives: &HashSet<u32>) -> Option<u32> {
    let end = ctx.comments.partition_point(|c| c.span.end <= start);
    for c in ctx.counted(ctx.comments[..end].iter().rev()) {
        // From the back, so code in between ends the scan before a long gap is read again.
        let gap = &ctx.text[c.span.end as usize..start as usize];
        let non_whitespace = gap.char_indices().rev().find(|&(_, ch)| !js_whitespace(ch));
        ctx.record_string_work(non_whitespace.map_or(gap.len(), |(index, _)| gap.len() - index));
        if non_whitespace.is_some() {
            return None;
        }
        if !directives.contains(&c.span.start) {
            let raw = c.content_span().source_text(ctx.text);
            return (c.is_block()
                && raw.starts_with('*')
                && !["@internal", "@private"].iter().any(|tag| {
                    ctx.record_string_work(raw.len());
                    raw.match_indices(tag)
                        .any(|(i, _)| word_end(&raw[i + tag.len()..]))
                }))
            .then_some(c.span.start);
        }
        start = c.span.start;
    }
    None
}

fn warning(ctx: &TsCtx<'_>, comment: &Comment, raw: &str) -> bool {
    if !comment.is_line() {
        return false;
    }
    ctx.record_string_work(raw.len());
    raw.trim_start_matches(js_whitespace)
        .strip_prefix("WARNING:")
        .is_some_and(|v| !v.trim_matches(js_whitespace).is_empty())
}

fn suppressed(ctx: &TsCtx<'_>, comment: &Comment) -> bool {
    let raw = comment.span.source_text(ctx.text);
    ctx.record_string_work(raw.len());
    suppressed_comment(raw)
}

fn check(ctx: &TsCtx<'_>, findings: &mut Findings) {
    if ctx.comments.is_empty() {
        return;
    }
    let directives: HashSet<_> = ctx
        .counted(ctx.comments.iter())
        .filter(|c| directive(ctx, c.content_span().source_text(ctx.text)) && !suppressed(ctx, c))
        .map(|c| c.span.start)
        .collect();
    let exposure = exposure(ctx);
    let documented: HashSet<_> = ctx
        .nodes_with_ids()
        .filter_map(|(id, n)| {
            let parent = ctx.semantic.nodes().parent_kind(id);
            let start = if let AstKind::ExportDefaultDeclaration(export) = parent
                && let Some(expr) = export.declaration.as_expression()
            {
                // ESTree elides parentheses but keeps TypeScript assertions as expression nodes.
                let inner = expr.without_parentheses();
                if matches!(inner, Expression::Identifier(_)) {
                    return None;
                }
                inner.span().start
            } else if exposure.can_document(id, n.kind())
                || matches!(
                    parent,
                    AstKind::ExportDeclaration(_) | AstKind::ExportDefaultDeclaration(_)
                )
            {
                n.span().start
            } else {
                return None;
            };
            doc_before(ctx, start, &directives)
        })
        .collect();
    for c in ctx.counted(ctx.comments.iter()) {
        if directives.contains(&c.span.start) {
            continue;
        }
        let allowed = documented.contains(&c.span.start)
            || warning(ctx, c, c.content_span().source_text(ctx.text));
        if !allowed || suppressed(ctx, c) {
            findings.report(c.span.start as usize, None);
        }
    }
}
