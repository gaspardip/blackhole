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
mod imports;
mod links;
mod module_object;
mod resolver;
mod specifier;
mod values;

use crate::engine::TsCtx;
use imports::{Import, Kind};
use oxc_resolver::PackageType;
pub(crate) use resolver::Reader;
use resolver::{Conditions, Loaders, Tree, TypeImportTarget};
use std::{
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
    bans: &'a [String],
}

impl<'a, 'source> RepoCtx<'a, 'source> {
    pub(crate) fn new(
        paths: Result<RepositoryPaths, String>,
        reader: &'a Reader<'source>,
        bans: &'a [String],
    ) -> Self {
        let resolution = paths.map(|paths| {
            let tree = Tree::from_paths(paths, reader);
            let loaders = tree.loaders();
            Resolution { tree, loaders }
        });
        Self { resolution, bans }
    }

    fn package_target(
        &self,
        importer: Result<&Path, &String>,
        mode: Conditions,
        name: &str,
    ) -> Result<Option<String>, String> {
        if !name.starts_with('#') {
            return Ok(None);
        }
        let path = importer.map_err(Clone::clone)?;
        let Resolution { tree, loaders } = self.resolution.as_ref().map_err(Clone::clone)?;
        let reader = tree.reader.expect("repository context has a reader");
        if let Some(loader) = loaders.types(mode)
            && let Some(target) = reader.type_import_target(loader, tree, mode, path, name)?
        {
            return Ok(match target {
                TypeImportTarget::Blocked | TypeImportTarget::Local => None,
                TypeImportTarget::Package(package) => Some(package),
            });
        }
        reader.package_target(tree, mode, path, name)
    }
}

#[derive(Clone, Copy)]
struct ModuleFormat {
    kind: PackageType,
    declared: bool,
}

impl ModuleFormat {
    fn has_filename_base(self, required: PackageType, extension: Option<&str>) -> bool {
        // Without a declared format, TypeScript tooling can change which loader runs the file.
        self.kind == required
            && (self.declared
                || match required {
                    PackageType::CommonJs => extension == Some("js"),
                    PackageType::Module => !matches!(extension, Some("ts" | "tsx")),
                })
    }
}

fn module_format(
    extension: Option<&str>,
    has_module_syntax: bool,
    package_type: impl FnOnce() -> Result<Option<PackageType>, String>,
) -> Result<ModuleFormat, String> {
    let explicit = match extension {
        Some("cjs" | "cts") => Some(PackageType::CommonJs),
        Some("mjs" | "mts") => Some(PackageType::Module),
        _ => None,
    };
    let declared = if explicit.is_some() {
        explicit
    } else {
        package_type()?
    };
    let implicit = if has_module_syntax {
        PackageType::Module
    } else {
        PackageType::CommonJs
    };
    Ok(ModuleFormat {
        kind: declared.unwrap_or(implicit),
        declared: declared.is_some(),
    })
}

fn loader_readings(
    kind: Kind,
    ts_extension: bool,
    module: Option<ModuleFormat>,
    name: &str,
) -> impl Iterator<Item = (Result<String, &'static str>, Conditions)> {
    use Conditions::{Import, Require, TypeImport, TypeRequire};
    let commonjs = module.is_some_and(|m| m.declared && m.kind == PackageType::CommonJs);
    let conditions = match (kind, commonjs) {
        (Kind::StaticRequire | Kind::Require, _) | (Kind::Static, true) => Require,
        (Kind::TypeRequire, _) | (Kind::TypeDeclaration, true) => TypeRequire,
        (Kind::TypeImport | Kind::TypeDeclaration, _) => TypeImport,
        _ => Import,
    };
    let dual = kind.implicit_mode()
        && !module.is_some_and(|m| m.declared)
        && ts_extension
        && specifier::bare(name)
        && (kind == Kind::TypeDeclaration || name.starts_with('#') || !name.contains('?'));
    specifier::readings(name, kind)
        .into_iter()
        .enumerate()
        .flat_map(move |(index, text)| {
            let other = (dual && text.is_ok() && (kind == Kind::TypeDeclaration || index == 0))
                .then(|| {
                    (
                        text.clone(),
                        if kind == Kind::TypeDeclaration {
                            TypeRequire
                        } else {
                            Require
                        },
                    )
                });
            std::iter::once((text, conditions)).chain(other)
        })
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

fn note(
    repo: &RepoCtx<'_, '_>,
    import: &Import,
    path: Result<&Path, &String>,
    module: Result<ModuleFormat, &String>,
    extension: Option<&str>,
    ts_extension: bool,
) -> Result<Option<String>, String> {
    let Some(name) = import.name.as_deref() else {
        return Ok(Some(NON_LITERAL.into()));
    };
    if let Some(required) = import.filename_base
        && module.is_ok_and(|m| !m.has_filename_base(required, extension))
    {
        return Ok(Some(NON_LITERAL.into()));
    }
    let needs_format = import.needs_format(ts_extension);
    for (reading, mode) in loader_readings(import.kind, ts_extension, module.ok(), name) {
        let name = reading?;
        if needs_format {
            module.map_err(Clone::clone)?;
        }
        let target = repo.package_target(path, mode, &name)?;
        if denied(&name, repo.bans) {
            return Ok(Some(name));
        }
        if target
            .as_deref()
            .is_some_and(|name| denied(name, repo.bans))
        {
            return Ok(target);
        }
    }
    Ok(None)
}

fn banned(ts: &TsCtx<'_>, repo: &RepoCtx<'_, '_>, findings: &mut Findings) {
    let imports = imports::collect(ts, ts.program.source_type.is_typescript_definition());
    if imports.is_empty() {
        return;
    }
    let rooted = Path::new("/").join(ts.path);
    let resolution = repo.resolution.as_ref();
    let importer = resolution
        .map_err(Clone::clone)
        .and_then(|resolution| resolution.tree.follow(&rooted));
    let extension = ts
        .path
        .rsplit('/')
        .next()
        .and_then(|name| name.rsplit_once('.'))
        .map(|(_, ext)| ext.to_ascii_lowercase());
    let extension = extension.as_deref();
    let ts_extension = matches!(extension, Some("ts" | "tsx"));
    let module = module_format(extension, ts.has_module_syntax, || {
        if imports.iter().any(|i| i.needs_format(ts_extension)) {
            let path = importer.as_deref().map_err(Clone::clone)?;
            resolution.map_err(Clone::clone)?.tree.package_scope(path)
        } else {
            Ok(None)
        }
    });
    let mut verbatim = None;
    let mut last = None;
    for import in &imports {
        ts.take_work(1);
        if last == Some(import.offset) {
            continue;
        }
        let Some(mut note) = note(
            repo,
            import,
            importer.as_deref(),
            module.as_ref().copied(),
            extension,
            ts_extension,
        )
        .unwrap_or_else(Some) else {
            continue;
        };
        if import.verbatim_only {
            match verbatim.get_or_insert_with(|| {
                let path = importer.as_deref().map_err(Clone::clone)?;
                let Resolution { tree, loaders } = resolution.map_err(Clone::clone)?;
                let reader = tree.reader.expect("repository context has a reader");
                reader.verbatim_module_syntax(loaders, path)
            }) {
                Ok(true) => {}
                Ok(false) => continue,
                Err(error) => note.clone_from(error),
            }
        }
        findings.report(import.offset as usize, Some(&note));
        last = Some(import.offset);
    }
}
