use crate::engine::{Family, FileCtx, Findings, Rule, Run};

pub(crate) const RULE: Rule = Rule {
    id: "core/no-suppression",
    family: Family::Judge,
    provenance: "Agents add suppression comments to make a failing check pass.",
    fix: "Fix what the tool reports instead of silencing it. If the tool is wrong, tell a person.",
    run: Run::File(check),
};

const PATTERNS: &[(&[&str], &[&str])] = &[
    (
        &[
            "js", "jsx", "mjs", "cjs", "ts", "tsx", "mts", "cts", "vue", "svelte", "astro",
        ],
        &[
            "eslint-disable",
            "@ts-ignore",
            "@ts-expect-error",
            "@ts-nocheck",
            "oxlint-disable",
            "biome-ignore",
            "deno-lint-ignore",
            "tslint:disable",
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
            "#~pyright~:~ignore",
            "#~pylint~:~disable",
            "#~pragma~:~no cover",
            "#~mypy~:~ignore-errors",
            "#~ruff~:~noqa",
            "#~flake8~:~noqa",
            "#~nosec",
        ],
    ),
    (
        &["go"],
        &[
            "//~nolint",
            "//~lint:ignore",
            "//~lint:file-ignore",
            "#~nosec",
        ],
    ),
    (&["rb"], &["rubocop:disable", "rubocop:todo", ":nocov:"]),
    (
        &["java", "kt", "kts"],
        &[
            "@suppresswarnings",
            "@suppress~(",
            "nosonar",
            "checkstyle:~off",
        ],
    ),
    (&["cs"], &["#~pragma warning disable", "suppressmessage"]),
    (
        &["c", "h", "cc", "cpp", "hpp"],
        &[
            "nolint",
            "#~pragma gcc diagnostic ignored",
            "#~pragma clang diagnostic ignored",
            "cppcheck-suppress",
            "lcov_excl",
        ],
    ),
    (&["swift"], &["swiftlint:disable"]),
    (
        &["php"],
        &[
            "@phpstan-ignore",
            "phpcs:ignore",
            "phpcs:disable",
            "@psalm-suppress",
            "@codecoverageignore",
        ],
    ),
    (&["sh", "bash"], &["shellcheck disable~="]),
    (&["css", "scss", "less"], &["stylelint-disable"]),
    (&["yaml", "yml"], &["yamllint disable"]),
    (&["dockerfile"], &["hadolint ignore~="]),
    (&["md", "mdx"], &["markdownlint-disable"]),
];

// A tilde permits optional whitespace; a space requires whitespace.
fn matches(text: &str, pattern: &str) -> bool {
    let mut chars = text.bytes().peekable();
    for token in pattern.bytes() {
        if matches!(token, b'~' | b' ') {
            let mut skipped = false;
            while chars.next_if(u8::is_ascii_whitespace).is_some() {
                skipped = true;
            }
            if token == b' ' && !skipped {
                return false;
            }
        } else if chars.next() != Some(token) {
            return false;
        }
    }
    true
}

fn rust(text: &str, hits: &mut Findings) {
    for (start, _) in text.match_indices('#') {
        let tail = text[start + 1..].trim_start();
        let tail = tail.strip_prefix('!').unwrap_or(tail).trim_start();
        let Some(body) = tail.strip_prefix('[').map(str::trim_start) else {
            continue;
        };
        if ["allow~(", "expect~(", "coverage~(~off~)"]
            .iter()
            .any(|p| matches(body, p))
        {
            hits.report(start, None);
        } else if matches(body, "cfg_attr~(") {
            cfg_attributes(text, body, hits);
        }
    }
}

fn cfg_attributes(text: &str, body: &str, hits: &mut Findings) {
    let mut depth = 0;
    for (i, c) in body.char_indices() {
        match c {
            ']' => break,
            '(' => depth += 1,
            ')' if depth <= 1 => break,
            ')' => depth -= 1,
            ',' => {
                let next = body[i + 1..].trim_start();
                if ["allow~(", "expect~("].iter().any(|p| matches(next, p)) {
                    hits.report(text.len() - next.len(), None);
                }
            }
            _ => {}
        }
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
        "sh" | "bash" => Some("sh"),
        "python" | "python2" | "python3" => Some("py"),
        "node" | "nodejs" => Some("js"),
        "ruby" => Some("rb"),
        "php" => Some("php"),
        _ => None,
    }
}

fn check(ctx: &FileCtx<'_>, hits: &mut Findings) {
    let path = std::path::Path::new(ctx.path);
    let ext = path.extension().and_then(std::ffi::OsStr::to_str);
    let kind = if path.file_name().is_some_and(|name| name == "Dockerfile") {
        "dockerfile"
    } else if let Some(ext) = ext {
        ext
    } else if let Some(kind) = script_kind(ctx.text) {
        kind
    } else {
        return;
    };
    let text = ctx.text.to_ascii_lowercase();
    if kind.eq_ignore_ascii_case("rs") {
        rust(&text, hits);
        return;
    }
    let Some((_, patterns)) = PATTERNS
        .iter()
        .find(|(exts, _)| exts.iter().any(|ext| kind.eq_ignore_ascii_case(ext)))
    else {
        return;
    };
    for pattern in *patterns {
        for (offset, _) in text.match_indices(&pattern[..1]) {
            if matches(&text[offset..], pattern) {
                if *pattern == "markdownlint-disable" {
                    let line = text[..offset].rsplit('\n').next().unwrap_or_default();
                    if line.rfind("<!--") <= line.rfind("-->") {
                        continue;
                    }
                }
                hits.report(offset, None);
            }
        }
    }
}
