use std::{
    ffi::OsString,
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
};
use tempfile::{TempDir, tempdir};

/// A `gh` on PATH that records every call and answers from canned responses keyed by method and endpoint.
/// An endpoint registered with `SHA` or `blackhole-canary-N` answers for any commit SHA or canary branch.
pub struct FakeGh {
    dir: TempDir,
}

const SCRIPT: &str = r#"#!/bin/sh
dir=@DIR@
printf '%s\n' "$*" >> "$dir/calls"
for last; do :; done
method=GET prev=
for arg; do
  [ "$prev" = --method ] && method=$arg
  prev=$arg
done
key() { printf '%s %s' "$method" "$1" | LC_ALL=C tr -c 'A-Za-z0-9._-' '_'; }
file="$dir/responses/$(key "$last")"
# A commit SHA or a canary branch name in the endpoint can be answered by one response for all of them.
[ -f "$file" ] || file="$dir/responses/$(key "$(printf '%s' "$last" | sed -E 's/[0-9a-f]{40}/SHA/g; s/blackhole-canary-[0-9]+/blackhole-canary-N/g')")"
if [ ! -f "$file" ]; then
  echo "fake gh: no response for $method $last" >&2
  exit 1
fi
[ ! -f "$file.err" ] || cat "$file.err" >&2
cat "$file"
exit "$(cat "$file.exit")"
"#;

fn key(method: &str, endpoint: &str) -> String {
    format!("{method} {endpoint}")
        .bytes()
        .map(|b| {
            if b.is_ascii_alphanumeric() || b"._-".contains(&b) {
                char::from(b)
            } else {
                '_'
            }
        })
        .collect()
}

fn sidecar(file: &Path, extension: &str) -> PathBuf {
    PathBuf::from(format!("{}.{extension}", file.display()))
}

impl FakeGh {
    /// # Panics
    /// Panics when the fake cannot be written.
    #[must_use]
    pub fn new() -> Self {
        let dir = tempdir().unwrap();
        fs::create_dir_all(dir.path().join("bin")).unwrap();
        fs::create_dir_all(dir.path().join("responses")).unwrap();
        fs::write(dir.path().join("calls"), "").unwrap();
        let quoted = format!("'{}'", dir.path().display());
        let gh = dir.path().join("bin/gh");
        fs::write(&gh, SCRIPT.replace("@DIR@", &quoted)).unwrap();
        fs::set_permissions(&gh, fs::Permissions::from_mode(0o755)).unwrap();
        Self { dir }
    }

    /// PATH with the fake first.
    ///
    /// # Panics
    /// Panics when PATH is unset.
    #[must_use]
    pub fn path(&self) -> OsString {
        let mut paths = vec![self.dir.path().join("bin")];
        paths.extend(std::env::split_paths(&std::env::var_os("PATH").unwrap()));
        std::env::join_paths(paths).unwrap()
    }

    fn file(&self, method: &str, endpoint: &str) -> PathBuf {
        self.dir
            .path()
            .join("responses")
            .join(key(method, endpoint))
    }

    /// Answers like `gh api -i`: status line, headers, a blank line and the body, exiting 1 on an error status as gh does.
    ///
    /// # Panics
    /// Panics when the response cannot be written.
    pub fn respond(
        &self,
        method: &str,
        endpoint: &str,
        status: u16,
        headers: &[(&str, &str)],
        body: &[u8],
    ) {
        let mut raw = format!("HTTP/2.0 {status} Canned\r\n").into_bytes();
        for (name, value) in headers {
            raw.extend_from_slice(format!("{name}: {value}\r\n").as_bytes());
        }
        raw.extend_from_slice(b"Content-Type: application/json\r\n\r\n");
        raw.extend_from_slice(body);
        let file = self.file(method, endpoint);
        fs::write(&file, raw).unwrap();
        let exit = if (200..300).contains(&status) {
            "0"
        } else {
            "1"
        };
        fs::write(sidecar(&file, "exit"), exit).unwrap();
        let _ = fs::remove_file(sidecar(&file, "err"));
    }

    /// Fails without a response, as gh does when it cannot reach GitHub or has no sign-in.
    ///
    /// # Panics
    /// Panics when the response cannot be written.
    pub fn fail(&self, method: &str, endpoint: &str, exit: i32, stderr: &str) {
        let file = self.file(method, endpoint);
        fs::write(&file, "").unwrap();
        fs::write(sidecar(&file, "exit"), exit.to_string()).unwrap();
        fs::write(sidecar(&file, "err"), stderr).unwrap();
    }

    /// # Panics
    /// Panics when the log cannot be read.
    #[must_use]
    pub fn calls(&self) -> Vec<String> {
        fs::read_to_string(self.dir.path().join("calls"))
            .unwrap()
            .lines()
            .map(str::to_owned)
            .collect()
    }
}

impl Default for FakeGh {
    fn default() -> Self {
        Self::new()
    }
}
