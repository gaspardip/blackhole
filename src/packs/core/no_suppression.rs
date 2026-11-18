use crate::engine::{Family, FileCtx, Findings, Rule, Run};

pub(crate) const RULE: Rule = Rule {
    id: "core/no-suppression",
    family: Family::Judge,
    provenance: "Agents add suppression comments to make a failing check pass.",
    fix: "Fix what the tool reports instead of silencing it. If the tool is wrong, tell a person.",
    run: Run::File(check),
};

// Patterns are lowercase. A tilde permits a gap, a space requires one, and a star is the rest of the line.
const PATTERNS: &[(&[&str], &[&str])] = &[
    (
        &[
            "js", "jsx", "mjs", "cjs", "ts", "tsx", "mts", "cts", "vue", "svelte", "astro",
        ],
        &[
            "eslint-disable",
            "/*~eslint ",
            "@ts-ignore",
            "@ts-expect-error",
            "@ts-nocheck",
            "oxlint-disable",
            "biome-ignore",
            "deno-lint-ignore",
            "deno-coverage-ignore",
            "tslint:disable",
            "$flowfixme",
            "$flowexpectederror",
            "istanbul ignore",
            "c8 ignore",
            "v8 ignore",
            "node:coverage",
            "/*~exported ",
            "/*~global ",
            "/*~globals ",
            "/*~eslint-env",
        ],
    ),
    (
        &["py", "pyi"],
        &[
            "#~noqa",
            "#~type~:~ignore",
            "#~pyright~:",
            "pylint~:~disable",
            "pylint~:~skip-file",
            "#~pragma~:~no~cover",
            "#~pragma~no~cover",
            "#~pragma~:~no~branch",
            "#~pragma~no~branch",
            "#~isort~:~skip",
            "#~isort~:~off",
            "#~mypy~:",
            "#~ruff~:",
            "#~flake8~:~noqa",
            "#~flake8~=~noqa",
            "#~nosec",
            "#~pyre-ignore",
            "#~pyre-fixme",
            "#~pytype~:~disable",
            "#~pytype~:~skip-file",
            "#~ty~:~ignore",
        ],
    ),
    (
        &["go"],
        &[
            "//~lint:ignore",
            "//~lint:file-ignore",
            "#~nosec",
            "revive:disable",
            "gosec:disable",
        ],
    ),
    (
        &["rb", "rake", "gemspec"],
        &[
            "rubocop~:~disable",
            "rubocop~:~todo",
            ":nocov:",
            "#~typed~:~ignore",
            "#~typed~:~false",
        ],
    ),
    (
        &["java", "kt", "kts"],
        &[
            "suppresswarnings",
            "suppress~(",
            "suppresslint",
            "suppressfbwarnings",
            "nosonar",
            "checkstyle:~off",
            "nopmd",
            "noinspection",
            "ktlint-disable",
        ],
    ),
    (
        &["cs"],
        &[
            "#~pragma warning disable",
            "suppressmessage",
            "#~nullable disable",
            "resharper disable",
            "excludefromcodecoverage",
        ],
    ),
    (
        &["c", "h", "cc", "cpp", "cxx", "hpp", "hh", "hxx", "m", "mm"],
        &[
            "nolint",
            "gcc diagnostic ignored",
            "clang diagnostic ignored",
            "gcc diagnostic warning",
            "clang diagnostic warning",
            "no_profile_instrument_function",
            "gcc system_header",
            "clang system_header",
            "warning~(~disable",
            "warning~(~suppress",
            "cppcheck-suppress",
            "lcov_excl",
            "gcov_excl",
            "gcovr_excl",
        ],
    ),
    (&["swift"], &["swiftlint:disable"]),
    (
        &["php"],
        &[
            "@phpstan-ignore",
            "phpcs:ignore",
            "phpcs:disable",
            "@codingstandardsignore",
            "@psalm-suppress",
            "@codecoverageignore",
            "suppresswarnings",
        ],
    ),
    (&["sh", "bash", "dash", "ksh"], &["shellcheck *disable~="]),
    (
        &["css", "scss", "less", "vue", "svelte", "astro"],
        &["stylelint-disable"],
    ),
    (
        &["yaml", "yml"],
        &[
            "yamllint disable",
            "#~noqa",
            "checkov:skip",
            "zizmor~:~ignore",
        ],
    ),
    (
        &["dockerfile"],
        &[
            "hadolint ignore~=",
            "hadolint global ignore~=",
            "#~check~=~skip",
        ],
    ),
    (
        &["md", "mdx"],
        &[
            "<!--~markdownlint-disable",
            "<!--~markdownlint-configure-file",
        ],
    ),
];

const ATTRIBUTES: &[&[&str]] = &[
    &["allow", "("],
    &["expect", "("],
    // Warn lowers a lint that the manifest denies; levels belong in the manifest.
    &["warn", "("],
    &["coverage", "(", "off"],
];

// The codecs that read ASCII text as it is written.
const READABLE: &[&str] = &[
    "utf-8",
    "utf8",
    "utf-8-sig",
    "ascii",
    "us-ascii",
    "latin-1",
    "latin1",
    "iso-8859-1",
    "iso8859-1",
    "cp1252",
    "windows-1252",
];

// No directive or attribute is longer than this, so a longer run is cut instead of rescanned from every start.
fn reach(text: &str) -> &str {
    &text[..text.floor_char_boundary(4096)]
}

// The text ran out, or ran past the window, before it could be read.
struct End;

// A C compiler joins a line ending in a backslash to the next one, and reads a comment as a space, before it reads a pragma.
fn skip(mut text: &str, gaps: bool) -> Result<&str, End> {
    loop {
        let mut rest = text
            .strip_prefix('\\')
            .or_else(|| text.strip_prefix("??/"))
            .map(|line| line.trim_start_matches([' ', '\t']))
            .and_then(|line| {
                line.strip_prefix("\r\n")
                    .or_else(|| line.strip_prefix(['\r', '\n']))
            })
            .unwrap_or(text);
        if gaps {
            // The tools read a byte order mark as whitespace too.
            rest = rest.trim_start_matches(|c: char| c.is_whitespace() || c == '\u{feff}');
            if let Some(inner) = rest.strip_prefix("/*") {
                let window = reach(inner);
                match window.find("*/") {
                    Some(end) => rest = &inner[end + 2..],
                    // A comment that closes past the window could hide the rest of a directive.
                    None if window.len() < inner.len() => return Err(End),
                    None => {}
                }
            }
        }
        if rest.len() == text.len() {
            return Ok(text);
        }
        text = rest;
    }
}

fn matches(text: &str, pattern: &str, kind: &str) -> Result<bool, End> {
    let mut rest = text;
    let mut tokens = pattern.chars();
    while let Some(token) = tokens.next() {
        if token == '*' {
            let window = reach(rest);
            let line = window.split('\n').next().unwrap_or(window);
            for (at, _) in line.char_indices() {
                if matches(&rest[at..], tokens.as_str(), kind)? {
                    return Ok(true);
                }
            }
            // A line that runs past the window could hold the rest of a directive.
            return if line.len() < rest.len() && line.len() == window.len() {
                Err(End)
            } else {
                Ok(false)
            };
        }
        let gaps = matches!(token, '~' | ' ');
        if matches!(kind, "java" | "cs") {
            // A name ends where its letters do, so these fall away around every token; a byte order mark before a gap is the gap.
            rest = rest.trim_start_matches(|c| ignorable(c, kind) && !(gaps && c == '\u{feff}'));
        }
        let after = skip(rest, gaps)?;
        if token == ' ' && after.len() == rest.len() {
            return Ok(false);
        }
        rest = after;
        if !gaps {
            let Some(after) = rest.strip_prefix(token) else {
                return Ok(false);
            };
            rest = after;
        }
    }
    Ok(true)
}

// Whitespace and comments may sit between any two tokens of an attribute.
fn trivia(mut text: &str) -> Result<&str, End> {
    loop {
        // Rust reads the two direction marks as whitespace too.
        text = text.trim_start_matches(|c: char| {
            c.is_whitespace() || matches!(c, '\u{200e}' | '\u{200f}')
        });
        if text.starts_with("//") {
            text = text.split_once('\n').ok_or(End)?.1;
        } else if let Some(mut rest) = text.strip_prefix("/*") {
            let mut depth = 1;
            while depth > 0 {
                if let Some(inner) = rest.strip_prefix("/*") {
                    depth += 1;
                    rest = inner;
                } else if let Some(outer) = rest.strip_prefix("*/") {
                    depth -= 1;
                    rest = outer;
                } else {
                    let mut chars = rest.chars();
                    chars.next().ok_or(End)?;
                    rest = chars.as_str();
                }
            }
            text = rest;
        } else if text.is_empty() {
            return Err(End);
        } else {
            return Ok(text);
        }
    }
}

fn tokens<'a>(mut text: &'a str, expected: &[&str]) -> Result<Option<&'a str>, End> {
    for token in expected {
        let rest = trivia(text)?;
        // A raw identifier names the same attribute.
        let Some(rest) = rest.strip_prefix("r#").unwrap_or(rest).strip_prefix(token) else {
            return Ok(None);
        };
        text = rest;
    }
    Ok(Some(text))
}

fn suppresses(attribute: &str) -> Result<bool, End> {
    for expected in ATTRIBUTES {
        if tokens(attribute, expected)?.is_some() {
            return Ok(true);
        }
    }
    Ok(false)
}

fn rust(text: &str, hits: &mut Findings) {
    for (start, _) in text.match_indices('#') {
        let rest = &text[start + 1..];
        let window = reach(rest);
        if attribute(start, window, hits).is_err() && window.len() < rest.len() {
            hits.report(start, Some("directive too long to read"));
        }
    }
}

fn attribute(start: usize, window: &str, hits: &mut Findings) -> Result<(), End> {
    let after = trivia(window)?;
    let Some(body) = tokens(after.strip_prefix('!').unwrap_or(after), &["["])? else {
        return Ok(());
    };
    let conditional = tokens(body, &["cfg_attr", "("])?.is_some();
    let mut positions = Vec::new();
    if suppresses(body)? {
        positions.push(start);
    }
    let mut depth = 1;
    let mut rest = body;
    while depth > 0 {
        rest = trivia(rest)?;
        let mut chars = rest.chars();
        match chars.next() {
            Some(c) if !c.is_ascii() => {
                hits.report(start, Some("unreadable attribute"));
                return Ok(());
            }
            Some('[') => depth += 1,
            Some(']') => depth -= 1,
            Some(',') if conditional => {
                let item = trivia(chars.as_str())?;
                if suppresses(item)? {
                    positions.push(start + 1 + window.len() - item.len());
                }
            }
            Some('\'') => {
                // A character literal closes; a lifetime does not.
                let mut ahead = chars.clone();
                match ahead.next() {
                    Some('\\') => {
                        ahead.next();
                        ahead.find(|&c| c == '\'').ok_or(End)?;
                        chars = ahead;
                    }
                    Some(_) if ahead.next() == Some('\'') => chars = ahead,
                    _ => {}
                }
            }
            Some('"') => {
                let before = &window[..window.len() - rest.len()];
                let prefix = before.trim_end_matches('#');
                let word = prefix.trim_end_matches(|c: char| c == '_' || c.is_alphanumeric());
                if matches!(&prefix[word.len()..], "r" | "br" | "cr") {
                    let close = format!("\"{}", &before[prefix.len()..]);
                    let inside = chars.as_str();
                    chars = inside[inside.find(&close).ok_or(End)? + close.len()..].chars();
                } else {
                    loop {
                        match chars.next().ok_or(End)? {
                            '\\' => {
                                chars.next();
                            }
                            '"' => break,
                            _ => {}
                        }
                    }
                }
            }
            Some(c) if c.is_ascii_alphanumeric() || c == '_' => {
                let tail = chars
                    .as_str()
                    .trim_start_matches(|c: char| c.is_ascii_alphanumeric() || c == '_');
                if matches!(
                    &rest[..rest.len() - tail.len()],
                    "tarpaulin" | "tarpaulin_include"
                ) && !positions.contains(&start)
                {
                    positions.push(start);
                }
                chars = tail.chars();
            }
            _ => {}
        }
        rest = chars.as_str();
    }
    for position in positions {
        hits.report(position, None);
    }
    Ok(())
}

// Python decodes a file with the codec its first two lines name, so an unknown codec hides the text from this check.
fn encoding(text: &str, hits: &mut Findings) {
    let mut offset = 0;
    for line in text.split_inclusive('\n').take(2) {
        let declared = line
            .match_indices("coding")
            .filter(|_| line.trim_start().starts_with('#'))
            .find_map(|(at, word)| {
                let name = line[at + word.len()..].strip_prefix([':', '='])?;
                let name = name.trim_start_matches([' ', '\t']);
                let end = name.find(|c: char| !(c.is_alphanumeric() || "-_.".contains(c)));
                Some((at, &name[..end.unwrap_or(name.len())]))
            });
        if let Some((at, name)) = declared
            && !READABLE.contains(&name.replace('_', "-").as_str())
        {
            hits.report(offset + at, Some("source encoding cannot be read"));
        }
        offset += line.len();
    }
}

// golangci-lint drops every leading slash and space of a comment before it looks for the directive.
fn nolint(text: &str, hits: &mut Findings) {
    for (start, _) in text.match_indices("//") {
        let rest = &text[start..];
        let window = reach(rest);
        let comment = window.trim_start_matches(|c: char| c == '/' || c.is_whitespace());
        if comment.starts_with("nolint") {
            hits.report(start, None);
        } else if window.len() < rest.len() && "nolint".starts_with(comment) {
            // The window ended before the word did.
            hits.report(start, Some("directive too long to read"));
        }
    }
}

// Java and C# drop these from a name: Unicode General_Category=Cf, and for Java some ISO controls as well.
fn ignorable(c: char, kind: &str) -> bool {
    matches!(u32::from(c), 0xad | 0x600..=0x605 | 0x61c | 0x6dd | 0x70f | 0x890..=0x891 |
        0x8e2 | 0x180e | 0x200b..=0x200f | 0x202a..=0x202e | 0x2060..=0x2064 |
        0x2066..=0x206f | 0xfeff | 0xfff9..=0xfffb | 0x110bd | 0x110cd |
        0x13430..=0x1343f | 0x1bca0..=0x1bca3 | 0x1d173..=0x1d17a |
        0xe0001 | 0xe0020..=0xe007f)
        || kind == "java" && matches!(u32::from(c), 0..=8 | 14..=27 | 127..=159)
}

// The character a Java or C# Unicode escape stands for, and the escape's length.
fn escape_unit(text: &str) -> Option<(u32, usize)> {
    let tail = text.strip_prefix('\\')?;
    let (digits, width) = match tail.strip_prefix('U') {
        Some(long) => (long, 8),
        None => (tail.strip_prefix('u')?.trim_start_matches('u'), 4),
    };
    let code = digits.get(..width)?;
    if !code.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let ch = u32::from_str_radix(code, 16).ok()?;
    Some((ch, text.len() - digits.len() + width))
}

fn escape(text: &str) -> Option<(char, usize)> {
    let (first, len) = escape_unit(text)?;
    if let Some(ch) = char::from_u32(first) {
        return Some((ch, len));
    }
    let (second, next) = escape_unit(&text[len..])?;
    if !(0xd800..=0xdbff).contains(&first) || !(0xdc00..=0xdfff).contains(&second) {
        return None;
    }
    let scalar = 0x10000 + ((first - 0xd800) << 10) + second - 0xdc00;
    Some((char::from_u32(scalar)?, len + next))
}

// Java and C# read an escape as the character itself before they read an annotation.
// Returns the translated text and, for each of its bytes, the offset it came from.
fn unescape(text: &str) -> (String, Vec<usize>) {
    let mut out = String::new();
    let mut origin = Vec::new();
    let mut rest = text;
    while let Some(first) = rest.chars().next() {
        let (piece, len) = if rest.starts_with("\\\\") {
            ("\\\\".to_owned(), 2)
        } else {
            let (ch, len) = escape(rest).unwrap_or((first, first.len_utf8()));
            (ch.to_string(), len)
        };
        out.push_str(&piece);
        origin.resize(out.len(), text.len() - rest.len());
        rest = &rest[len..];
    }
    (out, origin)
}

fn interpreter(word: &str) -> Option<&'static str> {
    let name = word.rsplit('/').next()?.to_ascii_lowercase();
    match name.trim_end_matches(|c: char| c.is_ascii_digit() || c == '.') {
        "sh" | "bash" | "dash" | "ksh" => Some("sh"),
        "python" | "pypy" => Some("py"),
        "node" | "nodejs" | "deno" | "bun" | "ts-node" | "tsx" => Some("js"),
        "ruby" => Some("rb"),
        "php" => Some("php"),
        _ => None,
    }
}

fn script_kinds(text: &str) -> Vec<&'static str> {
    let Some(line) = text.lines().next().and_then(|line| line.strip_prefix("#!")) else {
        return Vec::new();
    };
    let words = line
        .split(|c: char| c.is_whitespace() || matches!(c, '"' | '\''))
        .filter(|word| !word.is_empty());
    let mut args = words.clone();
    let env_options = args.next().and_then(|path| path.rsplit('/').next()) == Some("env")
        && args.any(|word| word.starts_with('-'));
    if !env_options {
        return words.clone().find_map(interpreter).into_iter().collect();
    }
    let mut kinds = Vec::new();
    for word in words {
        for piece in word.split("-S").flat_map(|piece| piece.split('=')) {
            if let Some(kind) = interpreter(piece)
                && !kinds.contains(&kind)
            {
                kinds.push(kind);
            }
        }
    }
    kinds
}

fn listed(kind: &str) -> impl Iterator<Item = &'static str> {
    PATTERNS
        .iter()
        .filter(move |(kinds, _)| kinds.contains(&kind))
        .flat_map(|(_, patterns)| patterns.iter().copied())
}

fn check(ctx: &FileCtx<'_>, hits: &mut Findings) {
    let path = ctx.path.to_ascii_lowercase();
    let name = path.rsplit('/').next().unwrap_or(&path);
    let known = |kind: &&str| *kind == "rs" || listed(kind).next().is_some();
    let kind = name
        .rsplit_once('.')
        .map(|(_, extension)| extension)
        .filter(known)
        // A known extension wins, or naming a source file after a Dockerfile would hide it.
        .or_else(|| {
            ["dockerfile", "containerfile"]
                .into_iter()
                .find(|image| {
                    name.strip_prefix(image)
                        .is_some_and(|rest| rest.is_empty() || rest.starts_with('.'))
                })
                .map(|_| "dockerfile")
        });
    if let Some(kind) = kind {
        check_kind(kind, ctx, hits);
    } else {
        for kind in script_kinds(ctx.text) {
            check_kind(kind, ctx, hits);
        }
    }
}

fn check_kind(kind: &str, ctx: &FileCtx<'_>, hits: &mut Findings) {
    if kind == "rs" {
        // Rust attributes are case-sensitive, and so is the prefix of a raw string.
        rust(ctx.text, hits);
        return;
    }
    let (mut text, origin) = if matches!(kind, "java" | "cs") {
        unescape(ctx.text)
    } else {
        (ctx.text.to_owned(), Vec::new())
    };
    text.make_ascii_lowercase();
    if matches!(kind, "py" | "pyi") {
        encoding(&text, hits);
    }
    if kind == "go" {
        nolint(&text, hits);
    }
    let patterns: Vec<_> = listed(kind).collect();
    for (offset, first) in text.char_indices() {
        let mut found = None;
        for pattern in patterns.iter().filter(|p| p.starts_with(first)) {
            match matches(&text[offset..], pattern, kind) {
                Ok(true) => {
                    found = Some(None);
                    break;
                }
                Ok(false) => {}
                Err(End) => found = Some(Some("directive too long to read")),
            }
        }
        if let Some(note) = found {
            hits.report(origin.get(offset).copied().unwrap_or(offset), note);
        }
    }
}
