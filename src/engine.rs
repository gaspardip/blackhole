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
}

pub struct FileCtx<'a> {
    pub path: &'a str,
    pub text: &'a str,
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
    &[crate::packs::core::no_suppression::RULE]
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

pub fn run<'a>(
    rules: &[Rule],
    files: impl IntoIterator<Item = (&'a str, &'a [u8])>,
) -> Vec<Finding> {
    let mut result = Vec::new();
    for (path, bytes) in files {
        let text = decode(bytes);
        let ctx = FileCtx { path, text: &text };
        let mut hits = Vec::new();
        for rule in rules {
            let Run::File(check) = rule.run;
            let mut findings = Findings::default();
            check(&ctx, &mut findings);
            hits.extend(
                findings
                    .0
                    .into_iter()
                    .map(|(offset, note)| (offset, rule.id, note)),
            );
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
