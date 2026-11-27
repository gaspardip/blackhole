use super::{
    Import, Importer, ReadingMode, RepoCtx, Resolution, collect_imports,
    importer::Target,
    resolver::{Charge, Conditions, repository_name},
    specifier, work_budget,
};
use crate::engine::{Family, Findings, Gate, Rule, Run, TsCtx, parse_dependency, source_type};
use globset::Candidate;
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
    sync::{Arc, Mutex, OnceLock},
};

pub const FOLDER: Rule = Rule {
    id: "boundary/folder-imports",
    family: Family::Boundary,
    gate: Gate::Advisory,
    provenance: "A direct dependency across a layer boundary couples its callers to the wrong layer.",
    fix: "Move the code or depend on the allowed layer. A re-export through another folder is the same violation. List workspace package names in deny to cover imports by name.",
    run: Run::Repo(folders),
};

type CachedExport = Arc<OnceLock<Result<Arc<ExportSource>, String>>>;
pub(super) type ExportCache = Mutex<BTreeMap<String, CachedExport>>;
type Targets = Mutex<BTreeMap<Conditions, BTreeMap<String, Target>>>;

pub(super) struct ExportSource {
    has_module_syntax: bool,
    javascript: bool,
    imports: Vec<Import>,
    targets: Targets,
}

impl RepoCtx<'_, '_> {
    fn exported_imports(
        &self,
        ts: &TsCtx<'_>,
        path: &str,
    ) -> Result<Option<Arc<ExportSource>>, String> {
        let Some(kind) = source_type(path) else {
            return Ok(None);
        };
        let cached = Arc::clone(
            self.exports
                .lock()
                .map_err(|_| "re-export cache lock poisoned")?
                .entry(path.to_owned())
                .or_default(),
        );
        cached
            .get_or_init(|| {
                let resolution = self.resolution.as_ref().map_err(Clone::clone)?;
                let reader = resolution
                    .tree
                    .reader
                    .expect("repository context has a reader");
                let bytes = reader
                    .read(path)
                    .map_err(|error| error.to_string())?
                    .ok_or_else(|| format!("{path:?}: listed dependency is unavailable"))?;
                parse_dependency(ts, path, &bytes, kind, |child| {
                    let mut imports = collect_imports(child);
                    imports.retain(Import::is_reexport);
                    Arc::new(ExportSource {
                        has_module_syntax: child.has_module_syntax,
                        javascript: kind.is_javascript(),
                        imports,
                        targets: Mutex::default(),
                    })
                })
            })
            .clone()
            .map(Some)
    }
}

impl Importer<'_, '_, '_> {
    fn read_folder(
        &self,
        import: &Import,
        matches: &impl Fn(&str) -> bool,
        pending: &mut Vec<(String, bool)>,
        type_only: bool,
        charge: &Charge<'_>,
        targets: Option<&Targets>,
    ) -> Result<Option<String>, String> {
        let path = self.path()?;
        let Resolution { tree, loaders } = self.resolution()?;
        if import.verbatim_only && !self.verbatim()? {
            return Ok(None);
        }
        for reading in self.readings(import, ReadingMode::Folder { type_only })? {
            charge()?;
            let (name, mode) = reading?;
            let resolve = || {
                self.package_target(&name, mode, charge, || {
                    tree.source_target(loaders, path, &name, mode, self.javascript, charge)
                })
            };
            let Target { file, package } = if let Some(targets) = targets {
                cached_target(targets, &name, mode, resolve)?
            } else {
                resolve()?
            };
            let written = specifier::unresolved(repository_name(path)?, &name);
            let target = file.as_deref().unwrap_or(&written);
            if matches(&name)
                || package.as_deref().is_some_and(matches)
                || (target != name && matches(target))
                || (specifier::relative(&name)
                    && written != target
                    && tree.files.contains(&Path::new("/").join(written.as_ref()))
                    && matches(&written))
            {
                return Ok(Some(target.to_owned()));
            }
            pending.extend(file.map(|path| (path, mode.is_type())));
        }
        Ok(None)
    }
}

fn cached_target(
    targets: &Targets,
    name: &str,
    mode: Conditions,
    resolve: impl FnOnce() -> Result<Target, String>,
) -> Result<Target, String> {
    let lock = || {
        targets
            .lock()
            .map_err(|_| "re-export target cache lock poisoned")
    };
    if let Some(target) = lock()?
        .get(&mode)
        .and_then(|names| names.get(name))
        .cloned()
    {
        return Ok(target);
    }
    let target = resolve()?;
    lock()?
        .entry(mode)
        .or_default()
        .insert(name.to_owned(), target.clone());
    Ok(target)
}

fn crossing(
    ts: &TsCtx<'_>,
    repo: &RepoCtx<'_, '_>,
    importer: &Importer<'_, '_, '_>,
    import: &Import,
    matches: &impl Fn(&str) -> bool,
    charge: &Charge<'_>,
) -> Result<Option<String>, String> {
    let mut pending = Vec::new();
    if let Some(note) = importer.read_folder(import, matches, &mut pending, false, charge, None)? {
        return Ok(Some(note));
    }
    let mut seen = BTreeSet::new();
    pending.reverse();
    while let Some((path, type_only)) = pending.pop() {
        charge()?;
        if !seen.insert((path.clone(), type_only)) {
            continue;
        }
        let Some(source) = repo.exported_imports(ts, &path)? else {
            continue;
        };
        if source.imports.is_empty() {
            continue;
        }
        let root = Path::new("/").join(&path);
        let site = Importer::new(
            &repo.resolution,
            &root,
            source.has_module_syntax,
            source.javascript,
        );
        site.path()?;
        let prior = pending.len();
        for dependency in &source.imports {
            charge()?;
            if let Some(note) = site.read_folder(
                dependency,
                matches,
                &mut pending,
                type_only,
                charge,
                Some(&source.targets),
            )? {
                return Ok(Some(note));
            }
        }
        pending[prior..].reverse();
    }
    Ok(None)
}

fn folders(ts: &TsCtx<'_>, repo: &RepoCtx<'_, '_>, findings: &mut Findings) {
    if repo.policy.folder_denies.is_empty() {
        return;
    }
    let origins = repo
        .policy
        .folder_origins
        .matches_candidate(&Candidate::new(ts.path));
    if origins.is_empty() {
        return;
    }
    let matches = |name: &str| {
        let candidate = Candidate::new(name);
        ts.counted(origins.iter())
            .any(|&origin| repo.policy.folder_denies[origin].is_match_candidate(&candidate))
    };
    let imports = ts.imports.get_or_init(|| collect_imports(ts));
    if imports.is_empty() {
        return;
    }
    let root = Path::new("/").join(ts.path);
    let importer = Importer::new(
        &repo.resolution,
        &root,
        ts.has_module_syntax,
        ts.program.source_type.is_javascript(),
    );
    let charge = work_budget(ts);
    let mut last = None;
    for import in imports {
        ts.take_work(1);
        if last == Some(import.offset) {
            continue;
        }
        if let Some(note) =
            crossing(ts, repo, &importer, import, &matches, &charge).unwrap_or_else(Some)
        {
            findings.report(import.offset as usize, Some(&note));
            last = Some(import.offset);
        }
    }
}
