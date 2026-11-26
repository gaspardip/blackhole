const EMPTY: &str = "specifier is empty";

pub(super) fn relative(name: &str) -> bool {
    name == "." || name == ".." || name.starts_with("./") || name.starts_with("../")
}

pub(super) fn bare(name: &str) -> bool {
    !relative(name) && !name.starts_with('/')
}

fn external_scheme(scheme: &str) -> bool {
    ["blob", "data", "http", "https"]
        .iter()
        .any(|s| scheme.eq_ignore_ascii_case(s))
}

pub(super) fn external_url(name: &str) -> bool {
    name.split_once(':')
        .is_some_and(|(scheme, _)| scheme.eq_ignore_ascii_case("node") || external_scheme(scheme))
}

pub(super) fn readings(name: &str, kind: super::Kind) -> Vec<Result<String, &'static str>> {
    if name.is_empty() {
        return vec![Err(EMPTY)];
    }
    if name.starts_with(|c| c <= ' ')
        || name.ends_with(|c| c <= ' ')
        || name.contains(['\t', '\n', '\r'])
    {
        return vec![Err("import specifier contains URL whitespace or controls")];
    }
    if let Some((scheme, rest)) = name.split_once(':') {
        if scheme.eq_ignore_ascii_case("file") {
            return vec![Err("file URL has no repository-relative target")];
        }
        if external_scheme(scheme) {
            return vec![Err("URL has no repository-relative target")];
        }
        if scheme.eq_ignore_ascii_case("node") && scheme != "node" {
            let canonical = format!("node:{rest}");
            let mut readings = readings(&canonical, kind);
            readings.insert(0, Ok(name.to_owned()));
            return readings;
        }
        if scheme
            .bytes()
            .next()
            .is_some_and(|b| b.is_ascii_alphabetic())
            && scheme
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'+' | b'.' | b'-'))
            && scheme != "node"
        {
            return vec![Ok(name.to_owned())];
        }
    }
    if !name.is_empty() && !name.contains(['\\', '%', '?', '#']) {
        return vec![Ok(name.to_owned())];
    }
    let mut readings = vec![Ok(name.to_owned())];
    if !kind.commonjs_call() {
        let url = url_reading(name);
        if url.as_deref() != Ok(name) {
            readings.push(url);
        }
    }
    readings
}

pub(super) fn path_end(name: &str) -> usize {
    let start = usize::from(name.starts_with('#'));
    name[start..]
        .find(['?', '#'])
        .map_or(name.len(), |i| start + i)
}

fn url_reading(name: &str) -> Result<String, &'static str> {
    let normalized = name.replace('\\', "/");
    let path = &normalized[..path_end(&normalized)];
    if path.is_empty() {
        return Err(EMPTY);
    }
    if !path.contains('%') {
        return Ok(path.to_owned());
    }
    let invalid = "unreadable percent escape in import specifier";
    let mut bytes = path.bytes();
    let mut decoded = Vec::with_capacity(path.len());
    while let Some(byte) = bytes.next() {
        if byte != b'%' {
            decoded.push(byte);
            continue;
        }
        let mut digit = || {
            bytes
                .next()
                .and_then(|c| char::from(c).to_digit(16))
                .ok_or(invalid)
        };
        let value = u8::try_from(digit()? * 16 + digit()?).map_err(|_| invalid)?;
        if matches!(value, 0 | b'/' | b'\\') {
            return Err(invalid);
        }
        decoded.push(value);
    }
    String::from_utf8(decoded).map_err(|_| invalid)
}

#[cfg(test)]
mod tests {
    use super::{super::Kind::*, readings};

    #[test]
    fn loader_readings() {
        let bad = Err(&"unreadable percent escape in import specifier");
        macro_rules! row {
            ($kind:expr, $name:expr, $($extra:expr),*) => {
                assert_eq!(readings($name, $kind).iter().map(Result::as_deref).collect::<Vec<_>>(), [Ok($name), $($extra),*], "{}", $name)
            }
        }
        row!(Require, "./%78.cjs",);
        row!(Require, "./x?raw",);
        row!(Static, "./%3Fx.mjs", Ok("./?x.mjs"));
        row!(Static, "./%23x.mjs", Ok("./#x.mjs"));
        row!(Static, "../db/%78.mjs", Ok("../db/x.mjs"));
        row!(Static, "./%2E", Ok("./."));
        row!(Dynamic, "./%2e", Ok("./."));
        row!(StaticRequire, "./%2F", bad);
        row!(TypeRequire, "./%5C", bad);
        row!(Static, "./%00", bad);
        row!(Static, "./%252E", Ok("./%2E"));
        row!(Static, "./%zz", bad);
        row!(Static, "./%", bad);
        row!(Static, "./x?raw", Ok("./x"));
        row!(Static, "./x#raw", Ok("./x"));
        row!(Static, "pkg/x.mjs?raw", Ok("pkg/x.mjs"));
        row!(Static, "pkg/x.mjs#raw", Ok("pkg/x.mjs"));
        row!(Static, "#choice",);
        row!(Static, "?only", Err(&"specifier is empty"));
        row!(Static, r".\x", Ok("./x"));
        row!(Static, r"./..\db\x.mjs", Ok("./../db/x.mjs"));
        row!(Static, "./alias.mjs",);
    }
}
