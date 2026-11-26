use crate::engine::{Family, Findings, Gate, Rule, Run};
use crate::files::{LEDGER_PATH, OBJECTIONS_PATH};
use oxc::syntax::line_terminator::LineTerminatorSplitter;

pub const SIZE: Rule = Rule {
    id: "boundary/file-size",
    family: Family::Boundary,
    gate: Gate::Blocking,
    provenance: "Growing unrelated responsibilities in one file obscures their boundaries.",
    fix: "Split the file along its responsibilities.",
    run: Run::Lines(check),
};

fn check(path: &str, text: &str, limit: usize, findings: &mut Findings) {
    if matches!(path, LEDGER_PATH | OBJECTIONS_PATH) {
        return;
    }
    let mut lines = LineTerminatorSplitter::new(text);
    if let Some(first_excess) = lines.nth(limit) {
        let offset = first_excess.as_ptr().addr() - text.as_ptr().addr();
        let total = limit + 1 + lines.count();
        findings.report(offset, Some(&format!("{total} lines, limit {limit}")));
    }
}

mod bases;
mod folder;
mod importer;
mod imports;
mod links;
mod module_object;
mod resolver;
mod specifier;
mod values;

use crate::{config::ImportPolicy, engine::TsCtx};
pub use folder::FOLDER;
use importer::{Importer, ReadingMode};
pub(crate) use imports::Import;
use imports::{Kind, collect as collect_imports};
pub(crate) use resolver::Reader;
use resolver::{Charge, Loaders, Tree};
use std::{
    cell::Cell,
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
};

pub const BANNED: Rule = Rule {
    id: "boundary/banned-imports",
    family: Family::Boundary,
    gate: Gate::Advisory,
    provenance: "A disallowed dependency can enter through any import form.",
    fix: "Remove the dependency and implement the operation with an allowed module.",
    run: Run::Repo(banned),
};

const NON_LITERAL: &str = "specifier is not a literal";

#[derive(Default)]
pub(crate) struct RepositoryPaths {
    pub files: BTreeSet<PathBuf>,
    pub links: BTreeMap<PathBuf, PathBuf>,
}

struct Resolution<'a, 'source> {
    tree: Tree<'a, 'source>,
    loaders: Loaders<'a, 'source>,
}

pub struct RepoCtx<'a, 'source> {
    resolution: Result<Resolution<'a, 'source>, String>,
    policy: ImportPolicy<'a>,
    exports: folder::ExportCache,
}

impl<'a, 'source> RepoCtx<'a, 'source> {
    pub(crate) fn new(
        paths: Result<RepositoryPaths, String>,
        reader: &'a Reader<'source>,
        policy: ImportPolicy<'a>,
    ) -> Self {
        let resolution = paths.map(|paths| {
            let tree = Tree::from_paths(paths, reader);
            let loaders = tree.loaders();
            Resolution { tree, loaders }
        });
        Self {
            resolution,
            policy,
            exports: folder::ExportCache::default(),
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum BuiltinSource {
    Module,
    Process,
}

impl BuiltinSource {
    fn source(name: Option<&str>) -> Option<Self> {
        match strip_node_scheme(name?) {
            "module" => Some(Self::Module),
            "process" => Some(Self::Process),
            _ => None,
        }
    }
}

fn strip_node_scheme(s: &str) -> &str {
    s.strip_prefix("node:").unwrap_or(s)
}

fn denied(name: &str, bans: &[String]) -> bool {
    bans.iter().any(|ban| {
        strip_node_scheme(name)
            .strip_prefix(strip_node_scheme(ban))
            .is_some_and(|rest| rest.is_empty() || rest.starts_with('/'))
    })
}

fn banned_note(
    site: &Importer<'_, '_, '_>,
    import: &Import,
    bans: &[String],
    charge: &Charge<'_>,
) -> Result<Option<String>, String> {
    for reading in site.readings(import, ReadingMode::Banned)? {
        let (name, mode) = reading?;
        let target = site.package_target(&name, mode, charge, || Ok(None))?;
        if denied(&name, bans) {
            return Ok(Some(name));
        }
        if target
            .package
            .as_deref()
            .is_some_and(|name| denied(name, bans))
        {
            return Ok(target.package);
        }
    }
    Ok(None)
}

fn banned(ts: &TsCtx<'_>, repo: &RepoCtx<'_, '_>, findings: &mut Findings) {
    if repo.policy.bans.is_empty() {
        return;
    }
    let imports = ts.imports.get_or_init(|| collect_imports(ts));
    if imports.is_empty() {
        return;
    }
    let rooted = Path::new("/").join(ts.path);
    let site = Importer::new(
        &repo.resolution,
        &rooted,
        ts.has_module_syntax,
        ts.program.source_type.is_javascript(),
    );
    site.prepare_banned(imports);
    let charge = work_budget(ts);
    let mut last = None;
    for import in imports {
        ts.take_work(1);
        if last == Some(import.offset) {
            continue;
        }
        let Some(mut note) =
            banned_note(&site, import, repo.policy.bans, &charge).unwrap_or_else(Some)
        else {
            continue;
        };
        if import.verbatim_only {
            match site.verbatim() {
                Ok(true) => {}
                Ok(false) => continue,
                Err(error) => note = error,
            }
        }
        findings.report(import.offset as usize, Some(&note));
        last = Some(import.offset);
    }
}

fn work_budget<'a>(ts: &'a TsCtx<'_>) -> impl Fn() -> Result<(), String> + 'a {
    let remaining = Cell::new(
        ts.semantic
            .nodes()
            .len()
            .saturating_mul(40)
            .saturating_add(1024),
    );
    move || {
        let next = remaining
            .get()
            .checked_sub(1)
            .ok_or_else(|| "package manifest resolution exceeded the work budget".to_owned())?;
        remaining.set(next);
        ts.record_work(1);
        Ok(())
    }
}
