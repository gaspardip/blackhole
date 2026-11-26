mod versions;

use super::{Conditions, Manifest, Tree, repository_name};
use oxc::span::SourceType;
use oxc_resolver::PathUtil;
use serde_json::Value;
use std::path::Path;

type Lookup = Result<bool, String>;
type TargetLookup<T> = Result<Option<T>, TargetError>;

pub(super) enum TargetError {
    InvalidTarget(String),
    Resolution(String),
}

pub(super) fn versioned_type_condition(name: &str) -> bool {
    name.strip_prefix("types@").is_some_and(versions::matches)
}

pub(super) fn is_package(name: &str) -> bool {
    super::super::specifier::bare(name)
        && !name.starts_with('#')
        && !super::super::specifier::external_url(name)
}

pub(super) fn import_target(
    tree: &Tree<'_, '_>,
    importer: &Path,
    name: &str,
    mode: Conditions,
) -> Result<Option<super::TypeImportTarget>, String> {
    if name == "#" || name.starts_with("#/") {
        return Ok(None);
    }
    let Some((root, manifest)) = Tree::nearest_manifest(importer, |path| tree.manifest(path))?
    else {
        return Ok(None);
    };
    let Some((target, capture)) = manifest
        .get("imports")
        .and_then(Value::as_object)
        .and_then(|paths| select_path(paths, name, true))
    else {
        return Ok(None);
    };
    let declarations = PackageLookup {
        tree,
        mode,
        javascript: false,
    };
    let javascript = PackageLookup {
        javascript: true,
        ..declarations
    };
    walk_target(target, mode, &mut |target| {
        let Some(target) = capture.apply(target) else {
            return Ok(None);
        };
        if let Some(path) = target.strip_prefix("./") {
            if invalid_relative(path) {
                return Err(TargetError::InvalidTarget(format!(
                    "invalid package import target {target:?} in {}",
                    root.display()
                )));
            }
            let path = root.join(path);
            let found = declarations
                .file(&path, false)
                .map_err(TargetError::Resolution)?
                || (!is_typescript(&path)
                    && javascript
                        .file(&path, false)
                        .map_err(TargetError::Resolution)?);
            return Ok(found.then_some(super::TypeImportTarget::Local));
        }
        if target.is_empty() || !is_package(&target) {
            return Ok(None);
        }
        lookup(tree, &root.join("package.json"), &target, mode)
            .map(|found| found.then_some(super::TypeImportTarget::Package(target)))
    })
    .map_err(TargetError::message)
}

fn lookup(
    tree: &Tree<'_, '_>,
    importer: &Path,
    name: &str,
    mode: Conditions,
) -> Result<bool, TargetError> {
    let package_end = name
        .match_indices('/')
        .nth(usize::from(name.starts_with('@')))
        .map_or(name.len(), |(index, _)| index);
    let (package, subpath) = name.split_at(package_end);
    let subpath = subpath.strip_prefix('/').unwrap_or(subpath);
    let package_lookup = PackageLookup {
        tree,
        mode,
        javascript: false,
    };
    let mut candidates = Vec::new();
    let mut invalid = None;
    if let Some((dir, manifest)) = Tree::nearest_manifest(importer, |path| tree.manifest(path))
        .map_err(TargetError::Resolution)?
        && manifest.get("name").and_then(Value::as_str) == Some(package)
        && let Some(exports) = exports_field(&manifest)
    {
        if remember_invalid(package_lookup.exports(dir, exports, subpath), &mut invalid)? {
            return Ok(true);
        }
        candidates.push((dir.to_owned(), Some(manifest)));
    }
    let empty = Manifest::new();
    for dir in importer.ancestors().skip(1) {
        if dir.file_name().is_some_and(|name| name == "node_modules") {
            continue;
        }
        let root = dir.join("node_modules").join(package);
        let real = tree.follow(&root).map_err(TargetError::Resolution)?;
        if !tree.dirs.contains(real.as_ref()) {
            continue;
        }
        let manifest = tree
            .manifest(&root.join("package.json"))
            .map_err(TargetError::Resolution)?;
        if remember_invalid(
            package_lookup.target(&root, manifest.as_deref().unwrap_or(&empty), subpath),
            &mut invalid,
        )? {
            return Ok(true);
        }
        candidates.push((root, manifest));
    }

    let javascript = PackageLookup {
        javascript: true,
        ..package_lookup
    };
    for (root, manifest) in candidates {
        if remember_invalid(
            javascript.target(&root, manifest.as_deref().unwrap_or(&empty), subpath),
            &mut invalid,
        )? {
            return Ok(true);
        }
    }
    invalid.map_or(Ok(false), Err)
}

fn exports_field(manifest: &Manifest) -> Option<&Value> {
    manifest.get("exports").filter(|value| !value.is_null())
}

struct PackageLookup<'a, 'tree, 'source> {
    tree: &'a Tree<'tree, 'source>,
    mode: Conditions,
    javascript: bool,
}

impl PackageLookup<'_, '_, '_> {
    fn target(&self, root: &Path, manifest: &Manifest, subpath: &str) -> Result<bool, TargetError> {
        if let Some(exports) = exports_field(manifest) {
            self.exports(root, exports, subpath)
        } else {
            self.legacy(root, manifest, subpath)
                .map_err(TargetError::Resolution)
        }
    }

    fn exports(&self, root: &Path, exports: &Value, subpath: &str) -> Result<bool, TargetError> {
        let paths = exports
            .as_object()
            .filter(|map| map.keys().any(|key| key.starts_with('.')));
        let key = if subpath.is_empty() {
            ".".to_owned()
        } else {
            format!("./{subpath}")
        };
        let selected = if let Some(paths) = paths {
            select_path(paths, &key, true)
        } else {
            subpath.is_empty().then_some((exports, Capture::Exact))
        };
        let Some((target, capture)) = selected else {
            return Ok(false);
        };
        walk_target(target, self.mode, &mut |target| {
            let invalid = |target: &str| {
                TargetError::InvalidTarget(format!(
                    "invalid package export target {target:?} in {}",
                    root.display()
                ))
            };
            let target = target.strip_prefix("./").ok_or_else(|| invalid(target))?;
            let Some(target) = capture.apply(target) else {
                return Ok(None);
            };
            if invalid_relative(&target) {
                return Err(invalid(&target));
            }
            self.file(&root.join(target), false)
                .map(|found| found.then_some(()))
                .map_err(TargetError::Resolution)
        })
        .map(|target| target.is_some())
    }

    fn legacy(&self, root: &Path, manifest: &Manifest, subpath: &str) -> Lookup {
        let fields: &[&str] = if self.javascript {
            &["main"]
        } else {
            &["typings", "types", "main"]
        };
        let rooted = |target: &str| root.join(target.replace('\\', "/")).normalize();
        let entry = fields
            .iter()
            .copied()
            .find_map(|field| {
                manifest
                    .get(field)
                    .and_then(Value::as_str)
                    .filter(|path| !path.is_empty())
            })
            .map(rooted);
        let candidate = if subpath.is_empty() {
            entry.clone().unwrap_or_else(|| root.join("index"))
        } else {
            rooted(subpath)
        };
        let module = manifest.get("type").and_then(Value::as_str) == Some("module");
        let implicit =
            !matches!(self.mode, Conditions::TypeImport) || (subpath.is_empty() && !module);
        if let Some((targets, capture)) = candidate
            .strip_prefix(root)
            .ok()
            .and_then(Path::to_str)
            .and_then(|path| {
                versions::paths(manifest).and_then(|paths| select_path(paths, path, false))
            })
        {
            for target in targets
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
            {
                let target = match capture {
                    Capture::Star(text) => target.replacen('*', text, 1),
                    _ => target.to_owned(),
                };
                if self.entry(&rooted(&target), implicit)? {
                    return Ok(true);
                }
            }
            if !matches!(self.mode, Conditions::TypeImport) || !subpath.is_empty() {
                return Ok(false);
            }
        } else if !subpath.is_empty() {
            return self.entry(&candidate, implicit);
        } else if entry.is_some() && self.entry(&candidate, implicit)? {
            return Ok(true);
        }
        self.file(&root.join("index.js"), false)
    }

    fn entry(&self, path: &Path, implicit: bool) -> Lookup {
        match self.file(path, implicit)? {
            false if implicit => self.file(&path.join("index"), true),
            found => Ok(found),
        }
    }

    fn suffixes(&self, extension: &str) -> &'static [&'static str] {
        let (declarations, javascript): (&[&str], &[&str]) = match (extension, self.javascript) {
            ("js", _) | ("ts", true) => (&[".ts", ".tsx", ".d.ts"], &[".js", ".jsx"]),
            ("jsx", _) | ("tsx", true) => (&[".tsx", ".ts", ".d.ts"], &[".jsx", ".js"]),
            ("mjs", _) | ("mts", true) => (&[".mts", ".d.mts"], &[".mjs"]),
            ("cjs", _) | ("cts", true) => (&[".cts", ".d.cts"], &[".cjs"]),
            _ => (&[], &[]),
        };
        if self.javascript {
            javascript
        } else {
            declarations
        }
    }

    fn file(&self, path: &Path, implicit: bool) -> Lookup {
        let name = repository_name(path)?;
        if name.ends_with('/') {
            return Ok(false);
        }
        if !self.javascript && is_typescript(path) && self.existing(path)? {
            return Ok(true);
        }
        let filename = path
            .file_name()
            .and_then(|file| file.to_str())
            .unwrap_or_default();
        if let Some((stem, extension)) = filename.rsplit_once('.') {
            let stem = match extension {
                "ts" | "mts" | "cts" if self.javascript => stem.strip_suffix(".d").unwrap_or(stem),
                _ => stem,
            };
            let suffixes = self.suffixes(extension);
            for suffix in suffixes {
                if self.existing(&path.with_file_name(format!("{stem}{suffix}")))? {
                    return Ok(true);
                }
            }
            if !self.javascript
                && !is_typescript(path)
                && suffixes.is_empty()
                && self.existing(&path.with_file_name(format!("{stem}.d.{extension}.ts")))?
            {
                return Ok(true);
            }
        }
        if implicit {
            for suffix in self.suffixes("js") {
                let mut candidate = path.as_os_str().to_os_string();
                candidate.push(suffix);
                if self.existing(Path::new(&candidate))? {
                    return Ok(true);
                }
            }
        }
        Ok(false)
    }

    fn existing(&self, path: &Path) -> Lookup {
        let real = self.tree.follow(path)?;
        let supported = if self.javascript {
            SourceType::from_path(&real).is_ok_and(SourceType::is_javascript)
        } else {
            is_typescript(&real)
        };
        if self.tree.files.contains(real.as_ref()) && supported {
            repository_name(&real)?;
            return Ok(true);
        }
        Ok(false)
    }
}

impl TargetError {
    pub(super) fn message(self) -> String {
        match self {
            Self::InvalidTarget(message) | Self::Resolution(message) => message,
        }
    }
}

fn remember_invalid(
    result: Result<bool, TargetError>,
    invalid: &mut Option<TargetError>,
) -> Result<bool, TargetError> {
    if let Err(error @ TargetError::InvalidTarget(_)) = result {
        invalid.get_or_insert(error);
        Ok(false)
    } else {
        result
    }
}

fn walk_target<T>(
    target: &Value,
    mode: Conditions,
    resolve: &mut impl FnMut(&str) -> TargetLookup<T>,
) -> TargetLookup<T> {
    let mut invalid = None;
    let mut try_target = |target| match walk_target(target, mode, resolve) {
        Err(error @ TargetError::InvalidTarget(_)) => {
            invalid.get_or_insert(error);
            None
        }
        Ok(None) => None,
        result => Some(result),
    };
    let result = match target {
        Value::String(target) => return resolve(target),
        Value::Object(conditions) => conditions
            .iter()
            .filter(|(key, _)| {
                mode.names().contains(&key.as_str()) || versioned_type_condition(key)
            })
            .find_map(|(_, target)| try_target(target)),
        Value::Array(targets) => targets.iter().find_map(try_target),
        _ => None,
    };
    result.unwrap_or_else(|| invalid.map_or(Ok(None), Err))
}

fn invalid_relative(target: &str) -> bool {
    target.starts_with('/')
        || target
            .split(['/', '\\'])
            .any(|part| matches!(part, "." | "..") || part.eq_ignore_ascii_case("node_modules"))
}

fn is_typescript(path: &Path) -> bool {
    SourceType::from_path(path).is_ok_and(SourceType::is_typescript)
}

#[derive(Clone, Copy)]
enum Capture<'a> {
    Exact,
    Star(&'a str),
    Tail(&'a str),
}

impl Capture<'_> {
    fn apply(self, target: &str) -> Option<String> {
        match self {
            Self::Exact => Some(target.to_owned()),
            Self::Star(text) => Some(target.replace('*', text)),
            Self::Tail(text) if target.ends_with('/') => Some(format!("{target}{text}")),
            Self::Tail(_) => None,
        }
    }
}

fn select_path<'a, 'b>(
    paths: &'a Manifest,
    name: &'b str,
    exports: bool,
) -> Option<(&'a Value, Capture<'b>)> {
    if (!exports || (!name.contains('*') && !name.ends_with('/')))
        && let Some(target) = paths.get(name)
    {
        return Some((target, Capture::Exact));
    }
    let mut best = None;
    let mut rank = None;
    for (key, target) in paths {
        let matched = if let Some((prefix, suffix)) = key.split_once('*') {
            if suffix.contains('*') {
                continue;
            }
            name.strip_prefix(prefix)
                .and_then(|text| text.strip_suffix(suffix))
                .map(|text| {
                    (
                        (
                            prefix.len() + usize::from(exports),
                            if exports { key.len() } else { 0 },
                        ),
                        Capture::Star(text),
                    )
                })
        } else if exports && key.ends_with('/') {
            name.strip_prefix(key)
                .map(|text| ((key.len(), 0), Capture::Tail(text)))
        } else {
            None
        };
        if let Some((priority, capture)) = matched
            && rank.is_none_or(|rank| priority > rank)
        {
            rank = Some(priority);
            best = Some((target, capture));
        }
    }
    best
}
