use oxc::{
    allocator::Allocator,
    ast::ast::{Comment, Program},
    diagnostics::{LabeledSpan, OxcDiagnostic},
    parser::{ParseOptions, Parser},
    semantic::{Semantic, SemanticBuilder},
    span::SourceType,
    syntax::line_terminator::is_line_terminator,
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
        let (pairs, tail) = bytes[2..].as_chunks();
        let units: Vec<_> = pairs
            .iter()
            .copied()
            .map(unit)
            .chain((!tail.is_empty()).then_some(0xfffd))
            .collect();
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
    // Without a primary span, the later token triggers the error and earlier tokens supply context.
    let at = error
        .labels
        .iter()
        .find(|label| label.primary())
        .or_else(|| error.labels.iter().max_by_key(|label| label.offset()))
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
        // TS1490 means the lexer encountered a standalone replacement character, but carries no span.
        if error.labels.is_empty()
            && error.code.scope.as_deref() == Some("TS")
            && error.code.number.as_deref() == Some("1490")
        {
            let mut replacements = text.match_indices(char::REPLACEMENT_CHARACTER);
            if let (Some((at, _)), None) = (replacements.next(), replacements.next()) {
                return Err((at, error.message.to_string()));
            }
        }
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

type Hit = (usize, &'static str, Option<String>);

fn collect(
    rules: &[Rule],
    path: &str,
    text: &str,
    parsed: Result<Option<&TsCtx<'_>>, &(usize, String)>,
) -> Vec<Hit> {
    let ts = parsed.ok().flatten();
    let ctx = FileCtx {
        path,
        text,
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
                if let Err((at, message)) = parsed {
                    findings.report(*at, Some(message));
                }
            }
        }
        hits.extend(findings.0.into_iter().map(|(at, note)| (at, rule.id, note)));
    }
    hits
}

fn parsed_hits(
    rules: &[Rule],
    path: &str,
    text: &str,
    kind: SourceType,
) -> Result<Vec<Hit>, (usize, String)> {
    // shortcut: a file nested deeper than this stack aborts the run (red, never green); judge files in a child process when a real repository hits it.
    const STACK_BYTES: usize = 768 * 1024 * 1024;
    std::thread::scope(|scope| {
        std::thread::Builder::new()
            .stack_size(STACK_BYTES)
            .spawn_scoped(scope, || {
                let arena = Allocator::default();
                let ts = parse(&arena, path, text, kind)?;
                Ok(collect(rules, path, text, Ok(Some(&ts))))
            })
            .map_err(|e| (0, format!("Cannot start parser thread: {e}")))?
            .join()
            .map_err(|_| (0, "Parser panicked".to_owned()))?
    })
}

pub fn run<'a>(
    rules: &[Rule],
    files: impl IntoIterator<Item = (&'a str, &'a [u8])>,
) -> Vec<Finding> {
    let mut result = Vec::new();
    for (path, bytes) in files {
        let (text, nul) = decode(bytes);
        let mut hits = match (source_type(path), nul) {
            (Some(_), Some(at)) => Err((at, "NUL in source text".to_owned())),
            (Some(kind), None) => parsed_hits(rules, path, &text, kind),
            (None, _) => Ok(collect(rules, path, &text, Ok(None))),
        }
        .unwrap_or_else(|error| collect(rules, path, &text, Err(&error)));
        hits.sort_by_key(|hit| hit.0);
        let (mut previous, mut line, mut col) = (0, 1, 1);
        for (offset, rule, note) in hits {
            for (at, ch) in text[previous..offset].char_indices() {
                // A carriage return before a line feed is the same break, counted at the line feed.
                let crlf = ch == '\r' && text[previous + at + 1..].starts_with('\n');
                if is_line_terminator(ch) && !crlf {
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

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn an_unlocated_diagnostic_starts_at_zero() {
        assert_eq!(
            diagnostic(&OxcDiagnostic::error("failure")),
            (0, "failure".to_owned())
        );
    }
}
