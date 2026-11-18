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
            "//~nolint",
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
    &["tarpaulin", "::", "skip"],
    &["cfg", "(", "not", "(", "tarpaulin"],
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

// A C compiler joins a line ending in a backslash to the next one, and reads a comment as a space, before it reads a pragma.
fn skip(mut text: &str, gaps: bool) -> &str {
    loop {
        let mut rest = ["\\\n", "\\\r\n", "??/\n", "??/\r\n"]
            .iter()
            .find_map(|splice| text.strip_prefix(splice))
            .unwrap_or(text);
        if gaps {
            // The tools read a byte order mark as whitespace too.
            rest = rest.trim_start_matches(|c: char| c.is_whitespace() || c == '\u{feff}');
            if let Some(inner) = rest.strip_prefix("/*")
                && let Some(end) = reach(inner).find("*/")
            {
                rest = &inner[end + 2..];
            }
        }
        if rest.len() == text.len() {
            return text;
        }
        text = rest;
    }
}

fn matches(text: &str, pattern: &str) -> bool {
    let mut rest = text;
    let mut tokens = pattern.chars();
    while let Some(token) = tokens.next() {
        if token == '*' {
            let line = reach(rest).split('\n').next().unwrap_or(rest);
            return line
                .char_indices()
                .any(|(at, _)| matches(&rest[at..], tokens.as_str()));
        }
        let gaps = matches!(token, '~' | ' ');
        let after = skip(rest, gaps);
        if token == ' ' && after.len() == rest.len() {
            return false;
        }
        rest = after;
        if !gaps {
            let Some(after) = rest.strip_prefix(token) else {
                return false;
            };
            rest = after;
        }
    }
    true
}

// The text ran out before the attribute could be read.
struct End;

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
            hits.report(start, Some("attribute too long to read"));
        }
    }
}

fn attribute(start: usize, window: &str, hits: &mut Findings) -> Result<(), End> {
    let after = trivia(window)?;
    let Some(body) = tokens(after.strip_prefix('!').unwrap_or(after), &["["])? else {
        return Ok(());
    };
    if suppresses(body)? {
        hits.report(start, None);
    } else if let Some(arguments) = tokens(body, &["cfg_attr", "("])? {
        conditional(start + 1, window, arguments, hits)?;
    }
    Ok(())
}

// `window` starts at byte `base` of the file and `arguments` is its tail after the opening parenthesis.
fn conditional(base: usize, window: &str, arguments: &str, hits: &mut Findings) -> Result<(), End> {
    let mut depth = 1;
    let mut rest = arguments;
    while depth > 0 {
        rest = trivia(rest)?;
        let mut chars = rest.chars();
        match chars.next() {
            Some('(') => depth += 1,
            Some(')') => depth -= 1,
            Some(',') => {
                let item = trivia(chars.as_str())?;
                if suppresses(item)? {
                    hits.report(base + window.len() - item.len(), None);
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
            _ => {}
        }
        rest = chars.as_str();
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

// The character a Java or C# Unicode escape stands for, and the escape's length.
fn escape(text: &str) -> Option<(char, usize)> {
    let tail = text.strip_prefix('\\')?;
    let (digits, width) = match tail.strip_prefix('U') {
        Some(long) => (long, 8),
        None => (tail.strip_prefix('u')?.trim_start_matches('u'), 4),
    };
    let code = digits.get(..width)?;
    if !code.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let ch = char::from_u32(u32::from_str_radix(code, 16).ok()?)?;
    Some((ch, text.len() - digits.len() + width))
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

fn script_kind(text: &str) -> Option<&'static str> {
    let line = text
        .lines()
        .next()?
        .strip_prefix("#!")?
        .to_ascii_lowercase();
    // Any word may be the interpreter: env takes options, assignments and a quoted command before it.
    line.split(|c: char| c.is_whitespace() || matches!(c, '"' | '\''))
        .find_map(|word| {
            let name = word.rsplit('/').next()?;
            match name.trim_end_matches(|c: char| c.is_ascii_digit() || c == '.') {
                "sh" | "bash" | "dash" | "ksh" => Some("sh"),
                "python" | "pypy" => Some("py"),
                "node" | "nodejs" | "deno" | "bun" | "ts-node" | "tsx" => Some("js"),
                "ruby" => Some("rb"),
                "php" => Some("php"),
                _ => None,
            }
        })
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
        })
        .or_else(|| script_kind(ctx.text));
    let Some(kind) = kind else { return };
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
    let patterns: Vec<_> = listed(kind).collect();
    for (offset, first) in text.char_indices() {
        if patterns
            .iter()
            .any(|p| p.starts_with(first) && matches(&text[offset..], p))
        {
            hits.report(origin.get(offset).copied().unwrap_or(offset), None);
        }
    }
}
