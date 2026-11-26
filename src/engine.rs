use std::cell::Cell;

use oxc::{
    allocator::Allocator,
    ast::ast::{Comment, Program},
    diagnostics::{LabeledSpan, OxcDiagnostic},
    parser::{ParseOptions, Parser},
    semantic::{AstNode, Semantic, SemanticBuilder},
    span::SourceType,
    syntax::{line_terminator::is_line_terminator, node::NodeId},
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

#[derive(Clone, Copy, Eq, PartialEq)]
pub enum Gate {
    Blocking,
    Advisory,
}

pub struct Rule {
    pub id: &'static str,
    pub family: Family,
    pub gate: Gate,
    pub provenance: &'static str,
    pub fix: &'static str,
    pub run: Run,
}

pub enum Run {
    File(fn(&FileCtx<'_>, &mut Findings)),
    Ts(fn(&TsCtx<'_>, &mut Findings)),
    TsConservative(fn(&TsCtx<'_>, &mut Findings)),
    Diff,
    ParseError,
}

impl Run {
    pub(crate) const fn can_allow(&self) -> bool {
        !matches!(self, Self::ParseError | Self::Diff)
    }
}

pub struct FileCtx<'a> {
    pub path: &'a str,
    pub text: &'a str,
    pub comments: Option<&'a [Comment]>,
}

pub struct TsCtx<'a> {
    work: Cell<u64>,
    pub(crate) arena: &'a Allocator,
    pub(crate) resolution: crate::packs::ts::values::Resolution<'a>,
    pub path: &'a str,
    pub text: &'a str,
    pub program: &'a Program<'a>,
    pub semantic: Semantic<'a>,
    pub comments: &'a [Comment],
}

impl<'a> TsCtx<'a> {
    pub(crate) fn record_work(&self, steps: usize) {
        self.work.set(self.work.get().saturating_add(steps as u64));
    }

    pub(crate) fn record_string_work(&self, bytes: usize) {
        self.record_work(bytes.max(1));
    }

    pub(crate) fn take_work(&self, steps: usize) -> bool {
        self.record_work(steps);
        self.resolution.take_work(steps)
    }

    pub(crate) fn take_string_work(&self, bytes: usize) -> bool {
        self.record_string_work(bytes);
        self.resolution.take_work(bytes.max(1))
    }

    pub(crate) fn counted<I: Iterator>(
        &self,
        items: I,
    ) -> std::iter::Inspect<I, impl FnMut(&I::Item)> {
        items.inspect(|_| self.record_work(1))
    }

    pub(crate) fn nodes(&self) -> impl Iterator<Item = &AstNode<'a>> {
        self.counted(self.semantic.nodes().iter())
    }

    pub(crate) fn nodes_with_ids(&self) -> impl Iterator<Item = (NodeId, &AstNode<'a>)> {
        self.nodes().map(|node| (node.id(), node))
    }

    pub(crate) fn ancestors(&self, id: NodeId) -> impl Iterator<Item = &AstNode<'a>> {
        self.counted(self.semantic.nodes().ancestors(id))
    }

    pub(crate) fn bounded_ancestors(&self, id: NodeId) -> impl Iterator<Item = &AstNode<'a>> {
        self.ancestors(id)
            .take_while(|_| self.resolution.take_work(1))
    }

    pub(crate) fn bounded_nodes(&self) -> impl Iterator<Item = &AstNode<'a>> {
        self.semantic
            .nodes()
            .iter()
            .take_while(|_| self.take_work(1))
    }
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
    pub gate: Gate,
    pub path: String,
    pub line: usize,
    pub col: usize,
    pub note: Option<String>,
}

const UNPARSEABLE: Rule = Rule {
    id: "core/unparseable",
    family: Family::Judge,
    gate: Gate::Blocking,
    provenance: "A syntax error would hide the file from parsed rules.",
    fix: "Make the file parse. The judge must be able to read it.",
    run: Run::ParseError,
};

#[must_use]
pub fn registry() -> &'static [Rule] {
    &[
        crate::packs::core::no_suppression::RULE,
        UNPARSEABLE,
        #[cfg(test)]
        crate::advisory_tests::RULE,
        crate::packs::ts::no_assertionless_tests::RULE,
        crate::packs::ts::no_chained_type_assertions::RULE,
        crate::packs::ts::no_comments::RULE,
        crate::packs::ts::no_conditional_empty_object_spread::RULE,
        crate::packs::ts::no_explicit_any::RULE,
        crate::packs::ts::no_explicit_return_types::RULE,
        crate::packs::ts::no_fixed_sleep::RULE,
        crate::packs::ts::no_flake_masking::RULE,
        crate::packs::ts::no_known_value_widening::RULE,
        crate::packs::ts::no_non_null_assertion::RULE,
        crate::packs::ts::no_object_parameters::RULE,
        crate::packs::ts::no_reduce_accumulator_copy::RULE,
        crate::packs::ts::no_reflect_apply::RULE,
        crate::packs::ts::no_reflect_get::RULE,
        crate::packs::ts::no_runtime_typeof::RULE,
        crate::packs::ts::no_shape_in_symbol_names::RULE,
        crate::packs::ts::no_skipped_tests::RULE,
        crate::packs::ts::no_test_aware_code::RULE,
        crate::packs::ts::no_test_doubles::RULE,
        crate::packs::ts::no_test_removal::RULE,
        crate::packs::ts::no_unknown_parameters::RULE,
        crate::packs::ts::no_unknown_returns::RULE,
        crate::packs::ts::no_unknown_type_aliases::RULE,
        crate::packs::ts::no_unsafe_dictionary_type::RULE,
        crate::packs::ts::no_unsignaled_poll::RULE,
        crate::packs::ts::no_widen_then_assert::RULE,
        crate::packs::ts::strict_msw::RULE,
    ]
}

// Bytes left over after the last whole unit are one more malformed unit.
fn decode_units(units: impl Iterator<Item = Option<char>>, tail: &[u8]) -> (String, Option<usize>) {
    let mut text = String::new();
    let mut malformed = None;
    for unit in units.chain((!tail.is_empty()).then_some(None)) {
        let ch = unit.unwrap_or_else(|| {
            malformed.get_or_insert(text.len());
            char::REPLACEMENT_CHARACTER
        });
        text.push(ch);
    }
    (text, malformed)
}

// A UTF-32LE mark starts with the UTF-16LE mark.
fn decode(bytes: &[u8]) -> (String, Option<usize>) {
    let wide = |unit: fn([u8; 2]) -> u16| {
        let (pairs, tail) = bytes[2..].as_chunks();
        let chars = char::decode_utf16(pairs.iter().copied().map(unit)).map(Result::ok);
        decode_units(chars, tail)
    };
    let utf32 = |unit: fn([u8; 4]) -> u32| {
        let (units, tail) = bytes[4..].as_chunks();
        decode_units(units.iter().copied().map(unit).map(char::from_u32), tail)
    };
    match bytes {
        [0xff, 0xfe, 0, 0, ..] => utf32(u32::from_le_bytes),
        [0, 0, 0xfe, 0xff, ..] => utf32(u32::from_be_bytes),
        [0xff, 0xfe, ..] => wide(u16::from_le_bytes),
        [0xfe, 0xff, ..] => wide(u16::from_be_bytes),
        _ => match std::str::from_utf8(bytes) {
            Ok(text) => (text.to_owned(), None),
            Err(e) => (
                String::from_utf8_lossy(bytes).into_owned(),
                Some(e.valid_up_to()),
            ),
        },
    }
}

pub(crate) fn can_allow(rules: &[Rule], id: &str) -> bool {
    rules.iter().any(|r| r.id == id && r.run.can_allow())
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
            && let Some(at) = text.find(char::REPLACEMENT_CHARACTER)
            && text.rfind(char::REPLACEMENT_CHARACTER) == Some(at)
        {
            return Err((at, error.message.to_string()));
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
        work: Cell::new(0),
        arena,
        resolution: crate::packs::ts::values::Resolution::default(),
        path,
        text,
        program,
        semantic: built.semantic,
        comments: &program.comments,
    })
}

pub(crate) fn source_type(path: &str) -> Option<SourceType> {
    let name = path.rsplit('/').next()?.to_ascii_lowercase();
    let kind = SourceType::from_path(&name)
        .ok()
        .or_else(|| SourceType::from_extension(name.strip_prefix('.')?).ok())?;
    // The TypeScript compiler reads JSX in every JavaScript file, whatever its extension.
    Some(kind.with_jsx(kind.is_jsx() || kind.is_javascript()))
}

pub(crate) fn parse_diagnostics(original: &str, path: &str, bytes: &[u8]) -> Vec<Finding> {
    let mut diagnostics = run(&[UNPARSEABLE], [(original, bytes)]);
    for finding in &mut diagnostics {
        path.clone_into(&mut finding.path);
    }
    diagnostics
}

pub(crate) fn with_parsed<T: Default + Send>(
    path: &str,
    original: &str,
    bytes: &[u8],
    visit: fn(&TsCtx<'_>) -> T,
) -> std::io::Result<T> {
    let Some(kind) = source_type(path).or_else(|| source_type(original)) else {
        return Ok(T::default());
    };
    let (text, malformed) = decode(bytes);
    if malformed.is_some() || text.contains('\0') {
        return Ok(T::default());
    }
    parser_thread(|| {
        let arena = Allocator::default();
        parse(&arena, path, &text, kind).map_or_else(|_| T::default(), |ts| visit(&ts))
    })?
    .map_err(|_| std::io::Error::other(format!("Parser panicked while counting tests in {path:?}")))
}

type Hit<'a> = (usize, &'a Rule, Option<String>);

fn collect<'r>(
    rules: &'r [Rule],
    path: &str,
    text: &str,
    parsed: Result<Option<&mut TsCtx<'_>>, &(usize, String)>,
) -> Vec<Hit<'r>> {
    let error = parsed.as_ref().err().copied();
    let mut ts = parsed.ok().flatten();
    let ctx = FileCtx {
        path,
        text,
        comments: ts.as_deref().map(|ts| ts.comments),
    };
    let mut hits = Vec::new();
    for rule in rules {
        let mut findings = Findings::default();
        match rule.run {
            Run::File(check) => check(&ctx, &mut findings),
            Run::Ts(check) => {
                if let Some(ts) = ts.as_deref() {
                    check(ts, &mut findings);
                }
            }
            Run::TsConservative(check) => {
                if let Some(ts) = ts.as_deref_mut() {
                    let resolution = std::mem::replace(
                        &mut ts.resolution,
                        crate::packs::ts::values::Resolution::conservative(),
                    );
                    check(ts, &mut findings);
                    ts.resolution = resolution;
                }
            }
            Run::ParseError => {
                if let Some((at, message)) = error {
                    findings.report(*at, Some(message));
                }
            }
            Run::Diff => (),
        }
        hits.extend(findings.0.into_iter().map(|(at, note)| (at, rule, note)));
    }
    hits
}

fn parser_thread<T: Send>(f: impl FnOnce() -> T + Send) -> std::io::Result<std::thread::Result<T>> {
    // shortcut: a file nested deeper than this stack aborts the run (red, never green); judge files in a child process when a real repository hits it.
    const STACK_BYTES: usize = 768 * 1024 * 1024;
    std::thread::scope(|scope| {
        Ok(std::thread::Builder::new()
            .stack_size(STACK_BYTES)
            .spawn_scoped(scope, f)?
            .join())
    })
}

fn parsed_hits<'r>(
    rules: &'r [Rule],
    path: &str,
    text: &str,
    kind: SourceType,
) -> Result<(Vec<Hit<'r>>, u64), (usize, String)> {
    parser_thread(|| {
        let arena = Allocator::default();
        let mut ts = parse(&arena, path, text, kind)?;
        let hits = collect(rules, path, text, Ok(Some(&mut ts)));
        Ok((hits, ts.work.get()))
    })
    .map_err(|e| (0, format!("Cannot start parser thread: {e}")))?
    .map_err(|_| (0, "Parser panicked".to_owned()))?
}

/// # Panics
/// Panics if a rule reports an offset past the end of its input.
pub fn run<'a>(
    rules: &[Rule],
    files: impl IntoIterator<Item = (&'a str, &'a [u8])>,
) -> Vec<Finding> {
    let mut result = Vec::new();
    for (path, bytes) in files {
        run_file(rules, path, bytes, &mut result);
    }
    sort_findings(&mut result);
    result
}

/// Runs the selected rules on one file and returns findings and counted work.
#[doc(hidden)]
#[must_use]
pub fn run_with_work(rules: &[Rule], path: &str, bytes: &[u8]) -> (Vec<Finding>, u64) {
    let mut findings = Vec::new();
    let work = run_file(rules, path, bytes, &mut findings);
    sort_findings(&mut findings);
    (findings, work)
}

fn sort_findings(findings: &mut [Finding]) {
    findings
        .sort_by(|a, b| (&a.path, a.line, a.col, a.rule).cmp(&(&b.path, b.line, b.col, b.rule)));
}

fn run_file(rules: &[Rule], path: &str, bytes: &[u8], result: &mut Vec<Finding>) -> u64 {
    let (decoded, malformed) = decode(bytes);
    let nul = decoded.find('\0');
    let stripped = nul.map(|_| decoded.replace('\0', ""));
    let text = stripped.as_deref().unwrap_or(&decoded);
    let (mut hits, work) = match (source_type(path), malformed, nul) {
        (Some(_), Some(at), _) => {
            let nuls = decoded[..at].bytes().filter(|&b| b == 0).count();
            Err((at - nuls, "Malformed source encoding".to_owned()))
        }
        (Some(_), None, Some(at)) => Err((at, "NUL in source text".to_owned())),
        (Some(kind), None, None) => parsed_hits(rules, path, text, kind),
        (None, _, _) => Ok((collect(rules, path, text, Ok(None)), 0)),
    }
    .unwrap_or_else(|error| (collect(rules, path, text, Err(&error)), 0));
    // A parse failure at the first NUL precedes matches at the same stripped offset.
    hits.sort_by_key(|(offset, rule, _)| (*offset, !matches!(rule.run, Run::ParseError)));
    let mut chars = decoded.chars().peekable();
    let (mut matched, mut line, mut col) = (0, 1, 1);
    for (offset, rule, note) in hits {
        let at_nul =
            malformed.is_none() && matches!(rule.run, Run::ParseError) && nul == Some(offset);
        while matched < offset || (!at_nul && chars.peek() == Some(&'\0')) {
            let ch = chars.next().expect("finding offset exceeds decoded text");
            if ch != '\0' {
                matched += ch.len_utf8();
            }
            if is_line_terminator(ch) && !(ch == '\r' && chars.peek() == Some(&'\n')) {
                line += 1;
                col = 1;
            } else {
                col += 1;
            }
        }
        result.push(Finding {
            rule: rule.id,
            gate: rule.gate,
            path: path.to_owned(),
            line,
            col,
            note,
        });
    }
    work
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
