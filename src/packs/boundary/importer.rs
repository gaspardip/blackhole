use super::{
    Import, Kind, NON_LITERAL, Resolution,
    resolver::{Charge, Conditions, TypeImportTarget},
    specifier,
};
use oxc_resolver::PackageType;
use std::{borrow::Cow, cell::OnceCell, path::Path};

#[derive(Clone, Copy)]
pub(super) enum ReadingMode {
    Banned,
    Folder { type_only: bool },
}

#[derive(Clone)]
pub(super) struct Target {
    pub file: Option<String>,
    pub package: Option<String>,
}

pub(super) struct Importer<'a, 'tree, 'source> {
    resolution: &'a Result<Resolution<'tree, 'source>, String>,
    path: Result<Cow<'a, Path>, String>,
    extension: Option<String>,
    module: OnceCell<Result<ModuleFormat, String>>,
    verbatim: OnceCell<Result<bool, String>>,
    has_module_syntax: bool,
    pub javascript: bool,
}

impl<'a, 'tree, 'source> Importer<'a, 'tree, 'source> {
    pub(super) fn new(
        resolution: &'a Result<Resolution<'tree, 'source>, String>,
        rooted: &'a Path,
        has_module_syntax: bool,
        javascript: bool,
    ) -> Self {
        Self {
            resolution,
            path: resolution
                .as_ref()
                .map_err(Clone::clone)
                .and_then(|resolution| resolution.tree.follow(rooted)),
            extension: rooted
                .file_name()
                .and_then(|name| name.to_str())
                .and_then(|name| name.rsplit_once('.'))
                .map(|(_, ext)| ext.to_ascii_lowercase()),
            module: OnceCell::new(),
            verbatim: OnceCell::new(),
            has_module_syntax,
            javascript,
        }
    }

    pub(super) fn resolution(&self) -> Result<&Resolution<'tree, 'source>, String> {
        self.resolution.as_ref().map_err(Clone::clone)
    }

    pub(super) fn path(&self) -> Result<&Path, String> {
        self.path.as_deref().map_err(Clone::clone)
    }

    fn ts_extension(&self) -> bool {
        matches!(self.extension.as_deref(), Some("ts" | "tsx"))
    }

    fn module(&self, scope: impl FnOnce() -> bool) -> Result<ModuleFormat, &String> {
        self.module
            .get_or_init(|| {
                module_format(self.extension.as_deref(), self.has_module_syntax, || {
                    if scope() {
                        let path = self.path()?;
                        self.resolution()?.tree.package_scope(path)
                    } else {
                        Ok(None)
                    }
                })
            })
            .as_ref()
            .copied()
    }

    pub(super) fn prepare_banned(&self, imports: &[Import]) {
        let _ = self.module(|| {
            imports
                .iter()
                .any(|import| import.needs_format(self.ts_extension()))
        });
    }

    pub(super) fn verbatim(&self) -> Result<bool, String> {
        self.verbatim
            .get_or_init(|| {
                let path = self.path()?;
                let Resolution { tree, loaders } = self.resolution()?;
                let reader = tree.reader.expect("repository context has a reader");
                reader.verbatim_module_syntax(loaders, path)
            })
            .clone()
    }

    pub(super) fn readings(
        &self,
        import: &Import,
        rule: ReadingMode,
    ) -> Result<impl Iterator<Item = Result<(String, Conditions), String>>, String> {
        let name = import.name.as_deref().ok_or(NON_LITERAL)?;
        let ts_extension = self.ts_extension();
        let module = self.module(|| true);
        let needs_format = import.needs_format(ts_extension);
        let kind = match rule {
            ReadingMode::Banned => import.kind,
            ReadingMode::Folder { type_only } => import.kind.in_type_context(type_only),
        };
        if matches!(rule, ReadingMode::Folder { .. })
            && (needs_format || kind.implicit_mode() && specifier::bare(name))
        {
            module.map_err(Clone::clone)?;
        }
        if let Some(required) = import.filename_base
            && module.is_ok_and(|m| !m.has_filename_base(required, self.extension.as_deref()))
        {
            return Err(NON_LITERAL.into());
        }
        Ok(
            loader_readings(kind, ts_extension, module.ok(), name).map(move |(name, mode)| {
                let name = name?;
                if needs_format {
                    module.map_err(Clone::clone)?;
                }
                Ok((name, mode))
            }),
        )
    }

    pub(super) fn package_target(
        &self,
        name: &str,
        mode: Conditions,
        charge: &Charge<'_>,
        source: impl FnOnce() -> Result<Option<String>, String>,
    ) -> Result<Target, String> {
        let mut package = None;
        if name.starts_with('#') {
            let path = self.path()?;
            let Resolution { tree, loaders } = self.resolution()?;
            let reader = tree.reader.expect("repository context has a reader");
            if let Some(loader) = loaders.types(mode)
                && let Some(target) =
                    reader.type_import_target(loader, tree, mode, path, name, charge)?
            {
                return Ok(match target {
                    TypeImportTarget::Blocked => Target {
                        file: None,
                        package: None,
                    },
                    TypeImportTarget::Local(file) => Target {
                        file: Some(file),
                        package: None,
                    },
                    TypeImportTarget::Package { name, file } => Target {
                        file: Some(file),
                        package: Some(name),
                    },
                });
            }
            package = reader.package_target(tree, mode, path, name, charge)?;
        }
        Ok(Target {
            file: source()?,
            package,
        })
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
