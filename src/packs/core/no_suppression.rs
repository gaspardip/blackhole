use crate::engine::{Family, FileCtx, Findings, Rule, Run};

pub(crate) const RULE: Rule = Rule {
    id: "core/no-suppression",
    family: Family::Judge,
    provenance: "Agents add suppression comments to make a failing check pass.",
    fix: "Fix what the tool reports instead of silencing it. If the tool is wrong, tell a person.",
    run: Run::File(check),
};

// Patterns are lowercase. A tilde permits a gap and a space requires one.
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
        ],
    ),
    (
        &["rb", "rake", "gemspec"],
        &[
            "rubocop~:~disable",
            "rubocop~:~todo",
            ":nocov:",
            "#~typed~:~ignore",
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
            "gcc system_header",
            "clang system_header",
            "warning~(~disable",
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
    (&["sh", "bash", "dash", "ksh"], &["shellcheck disable~="]),
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
    &["coverage", "(", "off", ")"],
    &["tarpaulin", "::", "skip"],
    &["cfg", "(", "not", "(", "tarpaulin_include"],
];

// A C compiler joins a line ending in a backslash to the next one before it reads a pragma.
fn skip(mut text: &str, gaps: bool) -> &str {
    loop {
        let rest = text
            .strip_prefix("\\\n")
            .or_else(|| text.strip_prefix("\\\r\n"))
            .unwrap_or(text);
        // The tools read a byte order mark as whitespace too.
        let rest =
            rest.trim_start_matches(|c: char| gaps && (c.is_whitespace() || c == '\u{feff}'));
        if rest.len() == text.len() {
            return text;
        }
        text = rest;
    }
}

fn matches(text: &str, pattern: &str) -> bool {
    let mut rest = text;
    for token in pattern.chars() {
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

// Whitespace and comments may sit between any two tokens of an attribute.
fn trivia(mut text: &str) -> &str {
    loop {
        text = text.trim_start();
        if text.starts_with("//") {
            text = text.split_once('\n').map_or("", |(_, rest)| rest);
        } else if let Some(mut rest) = text.strip_prefix("/*") {
            let mut depth = 1;
            while depth > 0 && !rest.is_empty() {
                if let Some(inner) = rest.strip_prefix("/*") {
                    depth += 1;
                    rest = inner;
                } else if let Some(outer) = rest.strip_prefix("*/") {
                    depth -= 1;
                    rest = outer;
                } else {
                    let mut chars = rest.chars();
                    chars.next();
                    rest = chars.as_str();
                }
            }
            text = rest;
        } else {
            return text;
        }
    }
}

fn tokens<'a>(mut text: &'a str, expected: &[&str]) -> Option<&'a str> {
    for token in expected {
        text = trivia(text).strip_prefix(token)?;
    }
    Some(text)
}

fn suppresses(attribute: &str) -> bool {
    ATTRIBUTES
        .iter()
        .any(|expected| tokens(attribute, expected).is_some())
}

fn rust(text: &str, hits: &mut Findings) {
    for (start, _) in text.match_indices('#') {
        let after = trivia(&text[start + 1..]);
        let Some(body) = tokens(after.strip_prefix('!').unwrap_or(after), &["["]) else {
            continue;
        };
        if suppresses(body) {
            hits.report(start, None);
        } else if let Some(arguments) = tokens(body, &["cfg_attr", "("]) {
            conditional(text, start, arguments, hits);
        }
    }
}

fn conditional(text: &str, start: usize, arguments: &str, hits: &mut Findings) {
    let mut depth = 1;
    let mut rest = arguments;
    while depth > 0 {
        rest = trivia(rest);
        let mut chars = rest.chars();
        match chars.next() {
            None => return,
            Some('(') => depth += 1,
            Some(')') => depth -= 1,
            Some(',') => {
                let item = trivia(chars.as_str());
                if suppresses(item) {
                    hits.report(text.len() - item.len(), None);
                }
            }
            Some('"') => {
                let before = &text[..text.len() - rest.len()];
                if before.ends_with(|c: char| c == '#' || c == '_' || c.is_alphanumeric()) {
                    // shortcut: a raw string hides where the attribute ends, so it is reported whole; lex raw strings when real code needs one here.
                    hits.report(start, Some("cfg_attr with a raw string cannot be read"));
                    return;
                }
                while let Some(c) = chars.next() {
                    match c {
                        '\\' => {
                            chars.next();
                        }
                        '"' => break,
                        _ => {}
                    }
                }
            }
            Some(_) => {}
        }
        rest = chars.as_str();
    }
}

fn script_kind(text: &str) -> Option<&'static str> {
    let mut words = text
        .lines()
        .next()?
        .strip_prefix("#!")?
        .split_ascii_whitespace();
    let first = words.next()?.rsplit('/').next()?;
    let interpreter = if first == "env" {
        words
            .find(|word| !word.starts_with('-') && !word.contains('='))?
            .rsplit('/')
            .next()?
    } else {
        first
    };
    match interpreter {
        "sh" | "bash" | "dash" | "ksh" => Some("sh"),
        "python" | "python2" | "python3" => Some("py"),
        "node" | "nodejs" => Some("js"),
        "ruby" => Some("rb"),
        "php" => Some("php"),
        _ => None,
    }
}

fn listed(kind: &str) -> impl Iterator<Item = &'static str> {
    PATTERNS
        .iter()
        .filter(move |(kinds, _)| kinds.contains(&kind))
        .flat_map(|(_, patterns)| patterns.iter().copied())
}

fn check(ctx: &FileCtx<'_>, hits: &mut Findings) {
    let text = ctx.text.to_ascii_lowercase();
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
        .or_else(|| script_kind(&text));
    let Some(kind) = kind else { return };
    if kind == "rs" {
        rust(&text, hits);
        return;
    }
    let patterns: Vec<_> = listed(kind).collect();
    for (offset, first) in text.char_indices() {
        if patterns
            .iter()
            .any(|p| p.starts_with(first) && matches(&text[offset..], p))
        {
            hits.report(offset, None);
        }
    }
}
