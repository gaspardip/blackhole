use std::{
    fmt::{self, Write},
    io,
    path::Path,
    process::{Command, Stdio},
};

/// The GitHub Actions app, the only source a required `blackhole` check may come from.
pub(crate) const ACTIONS_APP: u64 = 15368;
/// The name of the judge's check run and required status check.
pub(crate) const CHECK: &str = "blackhole";

pub(crate) struct Repo {
    pub(crate) host: String,
    pub(crate) slug: String,
}

/// The GitHub repository that the `origin` remote names, if it names one.
pub(crate) fn origin(root: &Path) -> io::Result<Option<Repo>> {
    let out = crate::git::command()
        .current_dir(root)
        .args(["config", "--get", "remote.origin.url"])
        .output()?;
    match out.status.code() {
        Some(0) => Ok(parse(String::from_utf8_lossy(&out.stdout).trim_end())),
        Some(1) => Ok(None),
        _ => Err(crate::git::failure(&out)),
    }
}

fn parse(url: &str) -> Option<Repo> {
    let (host, path) = match url.split_once("://") {
        Some((_, rest)) => rest.split_once('/')?,
        None => url.split_once(':')?,
    };
    let host = host.rsplit('@').next()?.split(':').next()?;
    let path = path.trim_end_matches('/');
    let (owner, name) = path.strip_suffix(".git").unwrap_or(path).split_once('/')?;
    let valid = |part: &str| {
        !part.is_empty()
            && part
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
    };
    (valid(host) && valid(owner) && valid(name)).then(|| Repo {
        host: host.to_owned(),
        slug: format!("{owner}/{name}"),
    })
}

/// Percent-encodes a branch name for a path segment or a query value.
pub(crate) fn encode(text: &str) -> String {
    let mut encoded = String::new();
    for b in text.bytes() {
        if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) {
            encoded.push(char::from(b));
        } else {
            let _ = write!(encoded, "%{b:02X}");
        }
    }
    encoded
}

pub(crate) enum Failure {
    /// gh is missing or has no usable sign-in.
    NoSession,
    /// GitHub could not answer for a reason outside the repository's settings.
    Unreachable(String),
}

impl fmt::Display for Failure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoSession => f.write_str("cannot read GitHub (install and sign in to gh)"),
            Self::Unreachable(e) => write!(f, "cannot read GitHub ({e})"),
        }
    }
}

pub(crate) struct Response {
    pub(crate) status: u16,
    headers: Vec<(String, String)>,
    pub(crate) body: Vec<u8>,
}

impl Response {
    pub(crate) fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    }

    pub(crate) fn json(&self) -> Option<serde_json::Value> {
        serde_json::from_slice(&self.body).ok()
    }
}

/// Calls the GitHub API through gh, which owns the sign-in. The endpoint is always the last argument.
pub(crate) fn api(
    host: &str,
    method: &str,
    endpoint: &str,
    args: &[&str],
) -> Result<Response, Failure> {
    let out = Command::new("gh")
        .args(["api", "--hostname", host, "--method", method, "-i"])
        .args(args)
        .arg(endpoint)
        .env("GH_PROMPT_DISABLED", "1")
        .env("GH_NO_UPDATE_NOTIFIER", "1")
        .stdin(Stdio::null())
        .output();
    let out = match out {
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Err(Failure::NoSession),
        Err(e) => return Err(Failure::Unreachable(format!("running gh: {e}"))),
        Ok(out) => out,
    };
    let Some(response) = response(&out.stdout) else {
        // gh exits 4 when it holds no credential for the host.
        if out.status.code() == Some(4) {
            return Err(Failure::NoSession);
        }
        let error = String::from_utf8_lossy(&out.stderr);
        let line = error.lines().next().unwrap_or("gh failed");
        return Err(Failure::Unreachable(
            crate::diagnostic_text(line).into_owned(),
        ));
    };
    match response.status {
        401 => Err(Failure::NoSession),
        429 | 500.. => Err(Failure::Unreachable(format!(
            "{endpoint} answered HTTP {}",
            response.status
        ))),
        403 if response.header("x-ratelimit-remaining") == Some("0") => {
            Err(Failure::Unreachable("rate limited".to_owned()))
        }
        _ => Ok(response),
    }
}

// gh -i prints the status line, the headers and a blank line before the body.
fn response(raw: &[u8]) -> Option<Response> {
    let mut offset = 0;
    let mut head: Vec<&str> = Vec::new();
    for line in raw.split_inclusive(|&b| b == b'\n') {
        offset += line.len();
        let line = line.strip_suffix(b"\n")?.trim_ascii_end();
        if line.is_empty() {
            let status = head.first()?;
            let status = status
                .strip_prefix("HTTP/")?
                .split(' ')
                .nth(1)?
                .parse()
                .ok()?;
            let headers = head[1..]
                .iter()
                .filter_map(|line| line.split_once(':'))
                .map(|(key, value)| (key.to_owned(), value.trim().to_owned()))
                .collect();
            return Some(Response {
                status,
                headers,
                body: raw[offset..].to_vec(),
            });
        }
        head.push(std::str::from_utf8(line).ok()?);
    }
    None
}

#[cfg(test)]
mod tests {
    #[test]
    fn remote_urls_name_their_repository() {
        for url in [
            "https://github.com/owner/repo.git",
            "https://user@github.com/owner/repo/",
            "git@github.com:owner/repo.git",
            "ssh://git@github.com:22/owner/repo",
        ] {
            let repo = super::parse(url).unwrap();
            assert_eq!(
                (repo.host.as_str(), repo.slug.as_str()),
                ("github.com", "owner/repo"),
                "{url}"
            );
        }
        for url in [
            "/srv/repo.git",
            "https://github.com/owner",
            "file:///owner/repo/x",
        ] {
            assert!(super::parse(url).is_none(), "{url}");
        }
    }

    #[test]
    fn responses_split_into_status_headers_and_body() {
        let raw = b"HTTP/2.0 404 Not Found\r\nX-Oauth-Scopes: repo, workflow\r\n\r\n{\"a\":1}\n";
        let response = super::response(raw).unwrap();
        assert_eq!(response.status, 404);
        assert_eq!(response.header("x-oauth-scopes"), Some("repo, workflow"));
        assert_eq!(response.body, b"{\"a\":1}\n");
        assert!(super::response(b"error connecting to api.github.com\n").is_none());
        assert_eq!(super::encode("release/1.0 x"), "release%2F1.0%20x");
    }
}
