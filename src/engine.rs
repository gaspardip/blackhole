use oxc::{
    allocator::Allocator,
    ast::ast::{Comment, Program},
    diagnostics::OxcDiagnostic,
    parser::{ParseOptions, Parser},
    semantic::{Semantic, SemanticBuilder},
    span::SourceType,
};

pub enum Family {
    Judge,
    Tests,
    Types,
    Narration,
    Slop,
    Structure,
    Boundary,
}

pub struct Rule {
    pub id: &'static str,
    pub family: Family,
    pub provenance: &'static str,
    pub fix: &'static str,
    pub run: Run,
}

pub enum Run {
    File(fn(&FileCtx<'_>, &mut Findings)),
    Ts(fn(&TsCtx<'_>, &mut Findings)),
    ParseError,
}

pub struct FileCtx<'a> {
    pub path: &'a str,
    pub text: &'a str,
    pub comments: Option<&'a [Comment]>,
}

pub struct TsCtx<'a> {
    pub path: &'a str,
    pub text: &'a str,
    pub program: &'a Program<'a>,
    pub semantic: &'a Semantic<'a>,
    pub comments: &'a [Comment],
}

#[derive(Default)]
pub struct Findings(Vec<(usize, Option<String>)>);

impl Findings {
    pub fn report(&mut self, offset: usize, note: Option<&str>) {
        self.0.push((offset, note.map(str::to_owned)));
    }
}

pub struct Finding {
    pub rule: &'static str,
    pub path: String,
    pub line: usize,
    pub col: usize,
    pub note: Option<String>,
}

#[must_use]
pub fn registry() -> &'static [Rule] {
    &[
        crate::packs::core::no_suppression::RULE,
        Rule {
            id: "core/unparseable",
            family: Family::Judge,
            provenance: "A syntax error would hide the file from parsed rules.",
            fix: "Make the file parse. The judge must be able to read it.",
            run: Run::ParseError,
        },
        crate::packs::ts::no_comments::RULE,
    ]
}

// TypeScript and C# read UTF-16 when a byte order mark says so, and UTF-32 is the same text with NULs in between.
fn decode(bytes: &[u8]) -> String {
    let wide = |unit: fn([u8; 2]) -> u16| {
        let units: Vec<_> = bytes[2..].as_chunks().0.iter().copied().map(unit).collect();
        String::from_utf16_lossy(&units)
    };
    let mut text = match bytes {
        [0xff, 0xfe, ..] => wide(u16::from_le_bytes),
        [0xfe, 0xff, ..] => wide(u16::from_be_bytes),
        _ => String::from_utf8_lossy(bytes).into_owned(),
    };
    text.retain(|c| c != '\0');
    text
}

pub(crate) fn can_allow(rules: &[Rule], id: &str) -> bool {
    rules
        .iter()
        .any(|r| r.id == id && !matches!(r.run, Run::ParseError))
}

type Hit = (usize, &'static str, Option<String>);

fn diagnostic(error: &OxcDiagnostic) -> (usize, &str) {
    (
        error
            .labels
            .first()
            .map_or(0, |label| label.offset() as usize),
        &error.message,
    )
}

fn parsed_hits(rules: &[Rule], path: &str, text: &str, kind: SourceType) -> Vec<Hit> {
    let arena = Allocator::default();
    let Ok(parsed) = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        Parser::new(&arena, text, kind)
            .with_options(ParseOptions {
                parse_regular_expression: true,
                ..ParseOptions::default()
            })
            .parse()
    })) else {
        return collect(rules, path, text, None, Some((0, "Parser panicked")));
    };
    if parsed.fatal_error || !parsed.diagnostics.is_empty() {
        let error = parsed
            .diagnostics
            .first()
            .map_or((0, "Parser failed"), diagnostic);
        return collect(rules, path, text, None, Some(error));
    }
    let built = SemanticBuilder::new()
        .with_build_nodes(true)
        .with_check_syntax_error(true)
        .build(&parsed.program);
    if let Some(error) = built.diagnostics.first() {
        return collect(rules, path, text, None, Some(diagnostic(error)));
    }
    let ts = TsCtx {
        path,
        text,
        program: &parsed.program,
        semantic: &built.semantic,
        comments: &parsed.program.comments,
    };
    collect(rules, path, text, Some(&ts), None)
}

fn collect(
    rules: &[Rule],
    path: &str,
    text: &str,
    ts: Option<&TsCtx<'_>>,
    error: Option<(usize, &str)>,
) -> Vec<Hit> {
    let ctx = FileCtx {
        path,
        text,
        comments: ts.map(|t| t.comments),
    };
    let mut hits = Vec::new();
    for rule in rules {
        let mut findings = Findings::default();
        match rule.run {
            Run::File(check) => check(&ctx, &mut findings),
            Run::Ts(check) => {
                if let Some(ts) = ts {
                    check(ts, &mut findings);
                }
            }
            Run::ParseError => {
                if let Some((at, message)) = error {
                    findings.report(at, Some(message));
                }
            }
        }
        hits.extend(findings.0.into_iter().map(|(at, note)| (at, rule.id, note)));
    }
    hits
}

fn source_type(path: &str) -> Option<SourceType> {
    let name = path.rsplit('/').next()?.to_ascii_lowercase();
    SourceType::from_path(&name)
        .ok()
        .or_else(|| SourceType::from_extension(name.strip_prefix('.')?).ok())
}

pub fn run<'a>(
    rules: &[Rule],
    files: impl IntoIterator<Item = (&'a str, &'a [u8])>,
) -> Vec<Finding> {
    let mut result = Vec::new();
    for (path, bytes) in files {
        let text = decode(bytes);
        let mut hits = match source_type(path) {
            Some(kind) => parsed_hits(rules, path, &text, kind),
            None => collect(rules, path, &text, None, None),
        };
        hits.sort_by_key(|hit| hit.0);
        let (mut previous, mut line, mut col) = (0, 1, 1);
        for (offset, rule, note) in hits {
            for (at, ch) in text[previous..offset].char_indices() {
                // A carriage return before a line feed is the same break, counted at the line feed.
                let alone = ch == '\r' && !text[previous + at + 1..].starts_with('\n');
                if alone || matches!(ch, '\n' | '\u{2028}' | '\u{2029}') {
                    line += 1;
                    col = 1;
                } else {
                    col += 1;
                }
            }
            previous = offset;
            result.push(Finding {
                rule,
                path: path.to_owned(),
                line,
                col,
                note,
            });
        }
    }
    result.sort_by(|a, b| (&a.path, a.line, a.col, a.rule).cmp(&(&b.path, b.line, b.col, b.rule)));
    result
}
