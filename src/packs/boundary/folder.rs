use super::{
    Import, ModuleFormat, NON_LITERAL, RepoCtx, Resolution, collect_imports, loader_readings,
    module_format,
    resolver::{Charge, Conditions, TypeImportTarget, repository_name},
    specifier, work_budget,
};
use crate::engine::{Family, Findings, Gate, Rule, Run, TsCtx, dependency_imports, source_type};
use globset::{Candidate, GlobSet};
use std::{borrow::Cow, cell::OnceCell, collections::BTreeSet, path::Path, sync::Arc};

pub const FOLDER: Rule = Rule {
    id: "boundary/folder-imports",
    family: Family::Boundary,
    gate: Gate::Advisory,
    provenance: "A direct dependency across a layer boundary couples its callers to the wrong layer.",
    fix: "Move the code or depend on the allowed layer. A re-export through another folder is the same violation. List workspace package names in deny to cover imports by name.",
    run: Run::Repo(folders),
};

pub(super) struct ExportSource {
    has_module_syntax: bool,
    javascript: bool,
    imports: Vec<Import>,
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
        let mut cache = self
            .exports
            .lock()
            .map_err(|_| "re-export cache lock poisoned")?;
        if let Some(source) = cache.get(path) {
            return Ok(Some(Arc::clone(source)));
        }
        let resolution = self.resolution.as_ref().map_err(Clone::clone)?;
        let reader = resolution
            .tree
            .reader
            .expect("repository context has a reader");
        let bytes = reader
            .read(path)
            .map_err(|error| error.to_string())?
            .ok_or_else(|| format!("{path:?}: listed dependency is unavailable"))?;
        let (has_module_syntax, imports) = dependency_imports(ts, path, &bytes, kind)?;
        let source = Arc::new(ExportSource {
            has_module_syntax,
            javascript: kind.is_javascript(),
            imports,
        });
        cache.insert(path.to_owned(), Arc::clone(&source));
        Ok(Some(source))
    }
}

struct Importer<'a, 'tree, 'source> {
    resolution: &'a Resolution<'tree, 'source>,
    path: Cow<'a, Path>,
    extension: Option<String>,
    module: OnceCell<Result<ModuleFormat, String>>,
    verbatim: OnceCell<Result<bool, String>>,
    has_module_syntax: bool,
    javascript: bool,
}

impl<'a, 'tree, 'source> Importer<'a, 'tree, 'source> {
    fn new(
        resolution: &'a Resolution<'tree, 'source>,
        rooted: &'a Path,
        has_module_syntax: bool,
        javascript: bool,
    ) -> Result<Self, String> {
        Ok(Self {
            resolution,
            path: resolution.tree.follow(rooted)?,
            extension: rooted
                .file_name()
                .and_then(|name| name.to_str())
                .and_then(|name| name.rsplit_once('.'))
                .map(|(_, ext)| ext.to_ascii_lowercase()),
            module: OnceCell::new(),
            verbatim: OnceCell::new(),
            has_module_syntax,
            javascript,
        })
    }

    fn read(
        &self,
        import: &Import,
        matches: &impl Fn(&str) -> bool,
        pending: &mut Vec<String>,
        charge: &Charge<'_>,
    ) -> Result<Option<String>, String> {
        let Resolution { tree, loaders } = self.resolution;
        let reader = tree.reader.expect("repository context has a reader");
        if import.verbatim_only
            && !*self
                .verbatim
                .get_or_init(|| reader.verbatim_module_syntax(loaders, &self.path))
                .as_ref()
                .map_err(Clone::clone)?
        {
            return Ok(None);
        }
        let name = import.name.as_deref().ok_or(NON_LITERAL)?;
        let extension = self.extension.as_deref();
        let module = self.module.get_or_init(|| {
            module_format(extension, self.has_module_syntax, || {
                tree.package_scope(&self.path)
            })
        });
        let ts_extension = matches!(extension, Some("ts" | "tsx"));
        if import.needs_format(ts_extension) || import.kind.implicit_mode() && specifier::bare(name)
        {
            module.as_ref().map_err(Clone::clone)?;
        }
        if let Some(required) = import.filename_base
            && module
                .as_ref()
                .is_ok_and(|m| !m.has_filename_base(required, extension))
        {
            return Err(NON_LITERAL.into());
        }
        for (reading, mode) in loader_readings(
            import.kind,
            ts_extension,
            module.as_ref().ok().copied(),
            name,
        ) {
            charge()?;
            let name = reading?;
            let (file, package) = self.target(&name, mode, charge)?;
            let written = specifier::unresolved(repository_name(&self.path)?, &name);
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
            pending.extend(file);
        }
        Ok(None)
    }

    fn target(
        &self,
        name: &str,
        mode: Conditions,
        charge: &Charge<'_>,
    ) -> Result<(Option<String>, Option<String>), String> {
        let Resolution { tree, loaders } = self.resolution;
        let reader = tree.reader.expect("repository context has a reader");
        let mut package = None;
        if name.starts_with('#') {
            if let Some(loader) = loaders.types(mode)
                && let Some(target) =
                    reader.type_import_target(loader, tree, mode, &self.path, name, charge)?
            {
                return Ok(match target {
                    TypeImportTarget::Blocked => (None, None),
                    TypeImportTarget::Local(file) => (Some(file), None),
                    TypeImportTarget::Package { name, file } => (Some(file), Some(name)),
                });
            }
            package = reader.package_target(tree, mode, &self.path, name, charge)?;
        }
        let file = tree.source_target(loaders, &self.path, name, mode, self.javascript, charge)?;
        Ok((file, package))
    }
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
    if let Some(note) = importer.read(import, matches, &mut pending, charge)? {
        return Ok(Some(note));
    }
    let mut seen = BTreeSet::new();
    pending.reverse();
    while let Some(path) = pending.pop() {
        charge()?;
        if !seen.insert(path.clone()) {
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
            importer.resolution,
            &root,
            source.has_module_syntax,
            source.javascript,
        )?;
        let prior = pending.len();
        for dependency in &source.imports {
            charge()?;
            if let Some(note) = site.read(dependency, matches, &mut pending, charge)? {
                return Ok(Some(note));
            }
        }
        pending[prior..].reverse();
    }
    Ok(None)
}

fn folders(ts: &TsCtx<'_>, repo: &RepoCtx<'_, '_>, findings: &mut Findings) {
    let origin = Candidate::new(ts.path);
    let denies: Vec<&GlobSet> = ts
        .counted(repo.policy.folders.iter())
        .filter(|folder| folder.from.is_match_candidate(&origin))
        .map(|folder| &folder.deny)
        .collect();
    if denies.is_empty() {
        return;
    }
    let matches = |name: &str| {
        let candidate = Candidate::new(name);
        ts.counted(denies.iter())
            .any(|deny| deny.is_match_candidate(&candidate))
    };
    let imports = ts.imports.get_or_init(|| collect_imports(ts));
    if imports.is_empty() {
        return;
    }
    let root = Path::new("/").join(ts.path);
    let importer = repo
        .resolution
        .as_ref()
        .map_err(Clone::clone)
        .and_then(|resolution| {
            Importer::new(
                resolution,
                &root,
                ts.has_module_syntax,
                ts.program.source_type.is_javascript(),
            )
        });
    let charge = work_budget(ts);
    let mut last = None;
    for import in ts.counted(imports.iter()) {
        if last == Some(import.offset) {
            continue;
        }
        if let Some(note) = importer
            .as_ref()
            .map_err(Clone::clone)
            .and_then(|site| crossing(ts, repo, site, import, &matches, &charge))
            .unwrap_or_else(Some)
        {
            findings.report(import.offset as usize, Some(&note));
            last = Some(import.offset);
        }
    }
}
