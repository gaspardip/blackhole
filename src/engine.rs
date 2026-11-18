use oxc::{
    allocator::Allocator,
    ast::ast::{Comment, Program},
    diagnostics::{LabeledSpan, OxcDiagnostic},
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
    pub semantic: Semantic<'a>,
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

// A UTF-32LE mark starts with the UTF-16LE mark.
fn decode(bytes: &[u8]) -> (String, Option<usize>) {
    let wide = |unit: fn([u8; 2]) -> u16| {
        let units: Vec<_> = bytes[2..].as_chunks().0.iter().copied().map(unit).collect();
        String::from_utf16_lossy(&units)
    };
    let utf32 = |unit: fn([u8; 4]) -> u32| {
        let (chars, tail) = bytes[4..].as_chunks();
        chars
            .iter()
            .copied()
            .map(unit)
            .map(|value| char::from_u32(value).unwrap_or(char::REPLACEMENT_CHARACTER))
            .chain((!tail.is_empty()).then_some(char::REPLACEMENT_CHARACTER))
            .collect()
    };
    let mut text = match bytes {
        [0xff, 0xfe, 0, 0, ..] => utf32(u32::from_le_bytes),
        [0, 0, 0xfe, 0xff, ..] => utf32(u32::from_be_bytes),
        [0xff, 0xfe, ..] => wide(u16::from_le_bytes),
        [0xfe, 0xff, ..] => wide(u16::from_be_bytes),
        _ => String::from_utf8_lossy(bytes).into_owned(),
    };
    let nul = text.find('\0');
    if nul.is_some() {
        text.retain(|c| c != '\0');
    }
    (text, nul)
}

pub(crate) fn can_allow(rules: &[Rule], id: &str) -> bool {
    rules
        .iter()
        .any(|r| r.id == id && !matches!(r.run, Run::ParseError))
}

fn diagnostic(error: &OxcDiagnostic) -> (usize, String) {
    // Oxc marks primary spans explicitly or puts the offending span after related declarations.
    let at = error
        .labels
        .iter()
        .find(|label| label.primary())
        .or_else(|| error.labels.last())
        .map_or(0, LabeledSpan::offset);
    (at as usize, error.message.to_string())
}

fn parse<'a>(
    arena: &'a Allocator,
    path: &'a str,
    text: &'a str,
    kind: SourceType,
) -> Result<TsCtx<'a>, (usize, String)> {
    let parsed = Parser::new(arena, text, kind)
        .with_options(ParseOptions {
            parse_regular_expression: true,
            ..ParseOptions::default()
        })
        .parse();
    if let Some(error) = parsed.diagnostics.first() {
        return Err(diagnostic(error));
    }
    if parsed.fatal_error {
        return Err((0, "Parser failed".to_owned()));
    }
    let program: &Program = arena.alloc(parsed.program);
    let built = SemanticBuilder::new()
        .with_build_nodes(true)
        .with_check_syntax_error(true)
        .build(program);
    if let Some(error) = built.diagnostics.first() {
        return Err(diagnostic(error));
    }
    Ok(TsCtx {
        path,
        text,
        program,
        semantic: built.semantic,
        comments: &program.comments,
    })
}

fn source_type(path: &str) -> Option<SourceType> {
    let name = path.rsplit('/').next()?.to_ascii_lowercase();
    let kind = SourceType::from_path(&name)
        .ok()
        .or_else(|| SourceType::from_extension(name.strip_prefix('.')?).ok())?;
    // The TypeScript compiler reads JSX in every JavaScript file, whatever its extension.
    Some(kind.with_jsx(kind.is_jsx() || kind.is_javascript()))
}

pub fn run<'a>(
    rules: &[Rule],
    files: impl IntoIterator<Item = (&'a str, &'a [u8])>,
) -> Vec<Finding> {
    let mut result = Vec::new();
    for (path, bytes) in files {
        let (text, nul) = decode(bytes);
        let arena = Allocator::default();
        // A parser that panics has not read the file, which is a finding like any other parse failure.
        let parsed = source_type(path).map(|kind| {
            if let Some(at) = nul {
                return Err((at, "NUL in source text".to_owned()));
            }
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                parse(&arena, path, &text, kind)
            }))
            .unwrap_or_else(|_| Err((0, "Parser panicked".to_owned())))
        });
        let ts = parsed.as_ref().and_then(|parsed| parsed.as_ref().ok());
        let ctx = FileCtx {
            path,
            text: &text,
            comments: ts.map(|ts| ts.comments),
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
                    if let Some(Err((at, message))) = &parsed {
                        findings.report(*at, Some(message));
                    }
                }
            }
            hits.extend(findings.0.into_iter().map(|(at, note)| (at, rule.id, note)));
        }
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
