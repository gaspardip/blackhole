use std::{collections::BTreeMap, io};

type Lines = BTreeMap<usize, bool>;
type Report<'a> = BTreeMap<&'a str, Lines>;

fn invalid() -> io::Error {
    io::Error::other("invalid or incomplete LCOV line coverage")
}

#[derive(Default)]
struct Record {
    lines: Lines,
    found: Option<usize>,
    hit: Option<usize>,
}

impl Record {
    fn field(&mut self, tag: &str, value: &str) -> io::Result<()> {
        match tag {
            "DA" => {
                let mut fields = value.split(',');
                let line: usize = fields
                    .next()
                    .ok_or_else(invalid)?
                    .parse()
                    .map_err(|_| invalid())?;
                let hits: u64 = fields
                    .next()
                    .ok_or_else(invalid)?
                    .parse()
                    .map_err(|_| invalid())?;
                if line == 0
                    || fields.next().is_some_and(str::is_empty)
                    || fields.next().is_some()
                    || self.lines.insert(line, hits > 0).is_some()
                {
                    return Err(invalid());
                }
            }
            "LF" | "LH" => {
                let slot = if tag == "LF" {
                    &mut self.found
                } else {
                    &mut self.hit
                };
                if slot
                    .replace(value.parse().map_err(|_| invalid())?)
                    .is_some()
                {
                    return Err(invalid());
                }
            }
            // Function, branch and MC/DC records do not determine line coverage.
            "VER" | "FN" | "FNDA" | "FNL" | "FNA" | "FNF" | "FNH" | "BRDA" | "BRF" | "BRH"
            | "MCDC" | "MRF" | "MRH" => {}
            _ => return Err(invalid()),
        }
        Ok(())
    }

    fn finish(self, dest: &mut Lines) -> io::Result<()> {
        if self.lines.is_empty() && self.found != Some(0)
            || self.found.is_some_and(|n| n != self.lines.len())
            || self
                .hit
                .is_some_and(|n| n != self.lines.values().filter(|&&hit| hit).count())
        {
            return Err(invalid());
        }
        for (line, hit) in self.lines {
            *dest.entry(line).or_default() |= hit;
        }
        Ok(())
    }
}

pub(super) fn parse(text: &str) -> io::Result<Report<'_>> {
    let mut result = Report::new();
    let mut current = None;
    for line in text.lines().filter(|line| !line.is_empty()) {
        if line == "end_of_record" {
            let (path, record): (_, Record) = current.take().ok_or_else(invalid)?;
            record.finish(result.entry(path).or_default())?;
            continue;
        }
        let (tag, value) = line.split_once(':').ok_or_else(invalid)?;
        match (tag, &mut current) {
            ("TN", None) => {}
            ("SF", None) => {
                let path = super::relative_path(value).ok_or_else(invalid)?;
                current = Some((path, Record::default()));
            }
            (_, Some((_, record))) => record.field(tag, value)?,
            _ => return Err(invalid()),
        }
    }
    if current.is_some() || result.is_empty() {
        return Err(invalid());
    }
    Ok(result)
}
