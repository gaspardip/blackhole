use std::{
    borrow::Cow,
    collections::{BTreeMap, BTreeSet},
    ffi::OsString,
    path::{Path, PathBuf},
};

#[derive(Default)]
pub(super) struct Links(BTreeMap<PathBuf, Option<PathBuf>>);

impl Links {
    pub fn new(
        raw: &BTreeMap<PathBuf, PathBuf>,
        files: &BTreeSet<PathBuf>,
        dirs: &BTreeSet<PathBuf>,
    ) -> Self {
        Self(
            raw.keys()
                .map(|name| (name.clone(), destination(name, raw, files, dirs)))
                .collect(),
        )
    }

    pub fn follow<'a>(&self, path: &'a Path) -> Result<Cow<'a, Path>, &Path> {
        let mut real = Cow::Borrowed(path);
        let mut hops = 0;
        while let Some((key, target)) = real.ancestors().find_map(|p| self.0.get_key_value(p)) {
            hops += 1;
            let target = target
                .as_deref()
                .filter(|_| hops <= 40)
                .ok_or(key.as_path())?;
            let suffix = real.strip_prefix(key).map_err(|_| key.as_path())?;
            real = Cow::Owned(if suffix.as_os_str().is_empty() {
                target.to_owned()
            } else {
                target.join(suffix)
            });
        }
        Ok(real)
    }
}

fn destination(
    name: &Path,
    raw: &BTreeMap<PathBuf, PathBuf>,
    files: &BTreeSet<PathBuf>,
    dirs: &BTreeSet<PathBuf>,
) -> Option<PathBuf> {
    let mut current = name.parent()?.to_owned();
    let mut pending = vec![name.file_name()?.to_owned()];
    let mut hops = 0;
    while let Some(part) = pending.pop() {
        if !dirs.contains(&current) {
            return None;
        }
        if part == ".." {
            if !current.pop() {
                return None;
            }
        } else if part != "." {
            current.push(part);
            if let Some(target) = raw.get(&current) {
                hops += 1;
                if hops > 40 || target.is_absolute() || target.as_os_str().is_empty() {
                    return None;
                }
                current.pop();
                let bytes = target.as_os_str().as_encoded_bytes();
                if bytes.ends_with(b"/") || bytes.ends_with(b"/.") {
                    pending.push(OsString::from("."));
                }
                pending.extend(
                    target
                        .components()
                        .rev()
                        .map(|c| OsString::from(c.as_os_str())),
                );
            }
        }
    }
    (files.contains(&current) || dirs.contains(&current)).then_some(current)
}
