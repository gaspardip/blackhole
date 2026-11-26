mod declarations;

use oxc_resolver::{
    FileMetadata, FileSystem, PackageJson, PackageType, ResolveError, ResolveOptions,
    ResolverGeneric, TsconfigDiscovery,
};
use serde_json::{Map, Value};
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
};

const MAX_READS: usize = 64;
// A Git path cannot collide with the virtual directory used to expose package specifiers.
const PACKAGE_TARGETS: &str = "/\0";
type ReadFile<'a> = Box<dyn FnMut(&str) -> io::Result<Option<Vec<u8>>> + Send + 'a>;
pub(crate) struct Reader<'a> {
    links: BTreeMap<PathBuf, PathBuf>,
    read: Mutex<ReadFile<'a>>,
    reads: AtomicUsize,
    failure: Mutex<Option<io::Error>>,
}
impl<'a> Reader<'a> {
    pub fn new(
        links: BTreeMap<PathBuf, PathBuf>,
        read: impl FnMut(&str) -> io::Result<Option<Vec<u8>>> + Send + 'a,
    ) -> Self {
        Self {
            links,
            read: Mutex::new(Box::new(read)),
            reads: AtomicUsize::new(0),
            failure: Mutex::default(),
        }
    }
    pub fn read(&self, name: &str) -> io::Result<Option<Vec<u8>>> {
        self.read
            .lock()
            .map_err(|_| io::Error::other("repository reader lock poisoned"))?(name)
        .map_err(|e| io::Error::new(e.kind(), format!("{name:?}: {e}")))
    }
    fn read_config(&self, name: &str) -> io::Result<Vec<u8>> {
        if self.reads.fetch_add(1, Ordering::Relaxed) >= MAX_READS {
            return Err(io::Error::other(Self::limit()));
        }
        self.read(name)
            .and_then(|bytes| {
                bytes.ok_or_else(|| {
                    io::Error::other(format!("{name:?}: listed configuration is unavailable"))
                })
            })
            .inspect_err(|e| self.record_failure(e))
    }
    fn record_failure(&self, error: &io::Error) {
        *self
            .failure
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) =
            Some(io::Error::new(error.kind(), error.to_string()));
    }
    fn limit() -> String {
        format!("resolution exceeded {MAX_READS} configuration reads")
    }
    pub(super) fn package_target<'tree>(
        &self,
        resolver: &Loader<'tree, 'a>,
        tree: &Tree<'tree, 'a>,
        mode: Conditions,
        file: &Path,
        name: &str,
    ) -> Result<Option<String>, String> {
        self.bounded(|| {
            let versioned = tree.versioned_imports_loader(resolver, mode, file, name)?;
            let resolver = versioned.as_ref().unwrap_or(resolver);
            Ok(resolver.resolve_file(file, name).ok().and_then(|target| {
                target
                    .path()
                    .strip_prefix(PACKAGE_TARGETS)
                    .ok()?
                    .to_str()
                    .map(str::to_owned)
            }))
        })
    }
    pub(super) fn type_import_target(
        &self,
        loader: &SourceLoader<'_, 'a>,
        tree: &Tree<'_, 'a>,
        mode: Conditions,
        file: &Path,
        name: &str,
    ) -> Result<Option<TypeImportTarget>, String> {
        self.bounded(|| {
            if Self::type_paths_target(loader, tree, file, name)?.is_some() {
                return Ok(Some(TypeImportTarget::Local));
            }
            declarations::import_target(tree, file, name, mode)
        })
    }
    fn type_paths_target(
        loader: &SourceLoader<'_, 'a>,
        tree: &Tree<'_, 'a>,
        file: &Path,
        name: &str,
    ) -> Result<Option<String>, String> {
        if loader
            .resolver
            .find_tsconfig(file)
            .map_err(|e| e.to_string())?
            .is_some()
        {
            resolve(&loader.aliases, tree, file, name)
        } else {
            Ok(None)
        }
    }
    fn bounded<T>(&self, lookup: impl FnOnce() -> Result<T, String>) -> Result<T, String> {
        self.reads.store(0, Ordering::Relaxed);
        let result = lookup();
        if self.reads.load(Ordering::Relaxed) > MAX_READS {
            Err(Self::limit())
        } else {
            result
        }
    }
    pub fn finish(self) -> io::Result<()> {
        self.failure
            .into_inner()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .map_or(Ok(()), Err)
    }
}

pub(super) enum TypeImportTarget {
    Local,
    Package(String),
}

type Manifest = Map<String, Value>;
type ManifestCache<T> = Mutex<BTreeMap<PathBuf, Result<Option<T>, String>>>;

#[derive(Clone, Default)]
pub(super) struct Tree<'a, 'source> {
    pub(super) files: Arc<BTreeSet<PathBuf>>,
    pub(super) links: Arc<super::links::Links>,
    dirs: Arc<BTreeSet<PathBuf>>,
    reader: Option<&'a Reader<'source>>,
    package_targets: bool,
    manifest_bytes: Arc<Mutex<BTreeMap<PathBuf, Vec<u8>>>>,
    manifests: Arc<ManifestCache<Arc<Manifest>>>,
    package_types: Arc<ManifestCache<Option<PackageType>>>,
}
impl<'a, 'source> Tree<'a, 'source> {
    pub fn from_files(names: BTreeSet<PathBuf>, reader: &'a Reader<'source>) -> Self {
        let dirs = names
            .iter()
            .chain(reader.links.keys())
            .flat_map(|p| p.ancestors().skip(1).map(Path::to_owned))
            .collect();
        Self {
            links: Arc::new(super::links::Links::new(&reader.links, &names, &dirs)),
            files: Arc::new(names),
            dirs: Arc::new(dirs),
            reader: Some(reader),
            ..Self::default()
        }
    }
    fn nearest_manifest<T>(
        file: &Path,
        mut read: impl FnMut(&Path) -> Result<Option<T>, String>,
    ) -> Result<Option<(&Path, T)>, String> {
        for dir in file
            .ancestors()
            .skip(1)
            .take_while(|dir| dir.file_name().is_none_or(|name| name != "node_modules"))
        {
            if let Some(manifest) = read(&dir.join("package.json"))? {
                return Ok(Some((dir, manifest)));
            }
        }
        Ok(None)
    }

    fn cached_manifest<T: Clone>(
        &self,
        path: &Path,
        cache: &ManifestCache<T>,
        parse: impl FnOnce(Vec<u8>) -> Result<T, String>,
    ) -> Result<Option<T>, String> {
        let mut cache = cache.lock().map_err(|_| "manifest cache lock poisoned")?;
        if let Some(manifest) = cache.get(path) {
            return manifest.clone();
        }
        let manifest = match self.read(path) {
            Ok(bytes) => parse(bytes).map(Some),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(e) => return Err(e.to_string()),
        };
        cache.insert(path.to_owned(), manifest.clone());
        manifest
    }

    fn manifest(&self, path: &Path) -> Result<Option<Arc<Manifest>>, String> {
        self.cached_manifest(path, &self.manifests, |bytes| {
            let json = bytes.strip_prefix(b"\xef\xbb\xbf").unwrap_or(&bytes);
            serde_json::from_slice(json)
                .map(Arc::new)
                .map_err(|e| format!("{}: {e}", path.display()))
        })
    }

    pub(super) fn package_scope(&self, file: &Path) -> Result<Option<PackageType>, String> {
        let lookup = || {
            Self::nearest_manifest(file, |path| {
                self.cached_manifest(path, &self.package_types, |bytes| {
                    PackageJson::parse(self, path.to_owned(), path.to_owned(), bytes)
                        .map(|manifest| manifest.r#type())
                        .map_err(|e| e.to_string())
                })
            })
            .map(|found| found.and_then(|(_, kind)| kind))
        };
        self.reader
            .map_or_else(lookup, |reader| reader.bounded(lookup))
    }

    fn versioned_imports_loader(
        &self,
        resolver: &Loader<'a, 'source>,
        mode: Conditions,
        file: &Path,
        name: &str,
    ) -> Result<Option<Loader<'a, 'source>>, String> {
        if !mode.is_type() || !name.starts_with('#') {
            return Ok(None);
        }
        let Some((_, manifest)) = Self::nearest_manifest(file, |path| self.manifest(path))? else {
            return Ok(None);
        };
        let mut names = Vec::new();
        if let Some(imports) = manifest.get("imports") {
            collect_versioned_conditions(imports, &mut names);
        }
        Ok((!names.is_empty()).then(|| {
            let mut options = resolver.options().clone();
            options.condition_names.extend(names);
            resolver.clone_with_options(options)
        }))
    }

    fn package_resolver(&self, options: &ResolveOptions) -> ResolverGeneric<Self> {
        ResolverGeneric::new_with_file_system(
            Self {
                package_targets: true,
                ..self.clone()
            },
            ResolveOptions {
                modules: vec![PACKAGE_TARGETS.to_owned()],
                resolve_to_context: true,
                tsconfig: None,
                ..options.clone()
            },
        )
    }
    pub(super) fn follow<'p>(&self, path: &'p Path) -> Result<std::borrow::Cow<'p, Path>, String> {
        self.links
            .follow(path)
            .map_err(|link| link_error(link).to_string())
    }

    pub(super) fn loaders(&self) -> Loaders<'a, 'source> {
        let resolver = self.resolver();
        let types = [Conditions::TypeImport, Conditions::TypeRequire].map(|mode| {
            let resolver = mode.resolver(&resolver);
            let mut options = resolver.options().clone();
            // Package entry points must go through declaration lookup even when a tsconfig exists.
            options.modules.clear();
            options.exports_fields.clear();
            options.imports_fields.clear();
            let aliases = resolver.clone_with_options(options);
            SourceLoader { resolver, aliases }
        });
        let packages = self.package_resolver(resolver.options());
        let packages = Conditions::ALL.map(|mode| mode.resolver(&packages));
        Loaders { types, packages }
    }
    fn resolver(&self) -> ResolverGeneric<Self> {
        ResolverGeneric::new_with_file_system(
            self.clone(),
            ResolveOptions {
                tsconfig: Some(TsconfigDiscovery::Auto),
                // The resolver validates package metadata before a linked entry point may leave its package.
                symlinks: false,
                ..ResolveOptions::default()
            },
        )
    }
}

fn collect_versioned_conditions(target: &Value, names: &mut Vec<String>) {
    match target {
        Value::Object(entries) => {
            for (name, target) in entries {
                if declarations::versioned_type_condition(name) {
                    names.push(name.clone());
                }
                collect_versioned_conditions(target, names);
            }
        }
        Value::Array(targets) => {
            for target in targets {
                collect_versioned_conditions(target, names);
            }
        }
        _ => {}
    }
}

type Loader<'tree, 'source> = ResolverGeneric<Tree<'tree, 'source>>;

pub(super) struct SourceLoader<'tree, 'source> {
    resolver: Loader<'tree, 'source>,
    aliases: Loader<'tree, 'source>,
}

pub(super) struct Loaders<'tree, 'source> {
    types: [SourceLoader<'tree, 'source>; 2],
    packages: [Loader<'tree, 'source>; 4],
}

impl<'tree, 'source> Loaders<'tree, 'source> {
    pub fn types(&self, mode: Conditions) -> Option<&SourceLoader<'tree, 'source>> {
        match mode {
            Conditions::TypeImport => Some(&self.types[0]),
            Conditions::TypeRequire => Some(&self.types[1]),
            _ => None,
        }
    }

    pub fn package(&self, mode: Conditions) -> &Loader<'tree, 'source> {
        &self.packages[mode as usize]
    }
}

const TS_EXTENSIONS: &[&str] = &[
    ".ts", ".tsx", ".mts", ".cts", ".js", ".jsx", ".mjs", ".cjs", ".json",
];
// TypeScript tries a declaration before JavaScript for an extensionless specifier.
const TYPE_EXTENSIONS: &[&str] = &[".ts", ".tsx", ".d.ts", ".js", ".jsx", ".json"];
const TS_ALIASES: &[(&str, &[&str])] = &[
    (".js", &[".ts", ".tsx", ".js"]),
    (".jsx", &[".tsx", ".jsx"]),
    (".mjs", &[".mts", ".mjs"]),
    (".cjs", &[".cts", ".cjs"]),
];
const TYPE_ALIASES: &[(&str, &[&str])] = &[
    (".js", &[".ts", ".tsx", ".d.ts", ".js"]),
    (".jsx", &[".tsx", ".ts", ".d.ts", ".jsx"]),
    (".mjs", &[".mts", ".d.mts", ".mjs"]),
    (".cjs", &[".cts", ".d.cts", ".cjs"]),
];

#[derive(Clone, Copy)]
pub(super) enum Conditions {
    Import,
    Require,
    TypeImport,
    TypeRequire,
}

impl Conditions {
    const ALL: [Self; 4] = [
        Self::Import,
        Self::Require,
        Self::TypeImport,
        Self::TypeRequire,
    ];

    pub(super) fn is_type(self) -> bool {
        matches!(self, Self::TypeImport | Self::TypeRequire)
    }

    fn resolver<'tree, 'source>(
        self,
        root: &ResolverGeneric<Tree<'tree, 'source>>,
    ) -> ResolverGeneric<Tree<'tree, 'source>> {
        let mut options = root.options().clone();
        let (extensions, aliases) = match self {
            Self::TypeImport | Self::TypeRequire => {
                options
                    .main_fields
                    .splice(..0, ["typings".into(), "types".into()]);
                (TYPE_EXTENSIONS, TYPE_ALIASES)
            }
            _ => (TS_EXTENSIONS, TS_ALIASES),
        };
        let strings = |values: &[&str]| values.iter().copied().map(str::to_owned).collect();
        options.extensions = strings(extensions);
        options.extension_alias = aliases
            .iter()
            .map(|(extension, candidates)| ((*extension).into(), strings(candidates)))
            .collect();
        options.condition_names = strings(self.names());
        root.clone_with_options(options)
    }

    fn names(self) -> &'static [&'static str] {
        match self {
            Self::Import => &["node", "import", "module-sync", "node-addons", "default"],
            Self::Require => &["require", "node", "node-addons", "module-sync", "default"],
            Self::TypeImport => &["types", "node", "import", "default"],
            Self::TypeRequire => &["types", "node", "require", "default"],
        }
    }
}

impl FileSystem for Tree<'_, '_> {
    fn new() -> Self {
        Self::default()
    }
    fn read(&self, path: &Path) -> io::Result<Vec<u8>> {
        if self.package_targets && path.starts_with(PACKAGE_TARGETS) {
            return Err(io::ErrorKind::NotFound.into());
        }
        let unreadable = |entry: &Path, reason: &str| {
            let entry = entry.strip_prefix("/").unwrap_or(entry);
            let message = format!("{:?}: {reason}", entry.display().to_string());
            if let Some(reader) = self.reader {
                reader.record_failure(&io::Error::other(message.clone()));
            }
            io::Error::other(message)
        };
        let real = self
            .links
            .follow(path)
            .map_err(|link| unreadable(link, "listed symbolic link is unreadable"))?;
        if self.dirs.contains(real.as_ref()) {
            return Err(unreadable(path, "listed configuration is a directory"));
        }
        if !self.files.contains(real.as_ref()) {
            return Err(io::ErrorKind::NotFound.into());
        }
        let name = repository_name(&real).map_err(io::Error::other)?;
        let reader = self.reader.ok_or(io::ErrorKind::NotFound)?;
        if path.file_name().is_none_or(|name| name != "package.json") {
            return reader.read_config(name);
        }
        let mut cache = self
            .manifest_bytes
            .lock()
            .map_err(|_| io::Error::other("manifest bytes cache lock poisoned"))?;
        if let Some(bytes) = cache.get(real.as_ref()) {
            return Ok(bytes.clone());
        }
        let bytes = reader.read_config(name)?;
        cache.insert(real.into_owned(), bytes.clone());
        Ok(bytes)
    }
    fn read_to_string(&self, path: &Path) -> io::Result<String> {
        String::from_utf8(self.read(path)?).map_err(io::Error::other)
    }
    fn metadata(&self, path: &Path) -> io::Result<FileMetadata> {
        if self.package_targets && path.starts_with(PACKAGE_TARGETS) {
            return Ok(FileMetadata::new(false, true, false));
        }
        let Ok(path) = self.links.follow(path) else {
            return Ok(FileMetadata::new(true, false, false));
        };
        let file = self.files.contains(path.as_ref());
        let dir = self.dirs.contains(path.as_ref());
        if file || dir {
            Ok(FileMetadata::new(file, dir, false))
        } else {
            Err(io::ErrorKind::NotFound.into())
        }
    }
    fn symlink_metadata(&self, path: &Path) -> io::Result<FileMetadata> {
        if self.reader.is_some_and(|r| r.links.contains_key(path)) {
            return Ok(FileMetadata::new(false, false, true));
        }
        self.metadata(path)
    }
    fn read_link(&self, path: &Path) -> Result<PathBuf, ResolveError> {
        self.reader
            .and_then(|r| r.links.get(path))
            .cloned()
            .ok_or_else(|| io::Error::from(io::ErrorKind::InvalidInput).into())
    }
    fn canonicalize(&self, path: &Path) -> io::Result<PathBuf> {
        let target = self.links.follow(path).map_err(link_error)?;
        self.metadata(&target).map(|_| target.into_owned())
    }
}

pub(super) fn repository_name(path: &Path) -> Result<&str, String> {
    path.strip_prefix("/")
        .ok()
        .and_then(Path::to_str)
        .ok_or_else(|| "invalid resolved repository path".to_owned())
}

fn link_error(link: &Path) -> io::Error {
    io::Error::other(format!(
        "unreadable repository link: {}",
        link.strip_prefix("/").unwrap_or(link).display()
    ))
}

fn resolve(
    resolver: &ResolverGeneric<Tree<'_, '_>>,
    tree: &Tree<'_, '_>,
    file: &Path,
    name: &str,
) -> Result<Option<String>, String> {
    let end = super::specifier::path_end(name);
    // This resolver removes escaped hashes by byte offset, which is only a character offset for ASCII.
    let escaped = if end < name.len() && name.is_ascii() {
        format!("{}{}", &name[..end], name[end..].replace('#', "\0#")).into()
    } else {
        std::borrow::Cow::Borrowed(name)
    };
    match resolver.resolve_file(file, &escaped) {
        Ok(module) if end < name.len() && module.fragment().is_some() => Ok(None),
        Ok(module) => Ok(Some(
            repository_name(&tree.follow(module.path())?)?.to_owned(),
        )),
        Err(
            ResolveError::NotFound(_)
            | ResolveError::ExtensionAlias(..)
            | ResolveError::MatchedAliasNotFound(..)
            | ResolveError::PackageImportNotDefined(..)
            | ResolveError::PackagePathNotExported { .. },
        ) => Ok(None),
        Err(e) => Err(e.to_string()),
    }
}
