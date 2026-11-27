use std::io;

const END: &str = "*** End Patch";

#[derive(Clone, Copy, PartialEq, Eq)]
enum Chunk {
    Start,
    Moved,
    NeedsLine,
    Body,
    End,
}

#[derive(Clone, Copy)]
enum Hunk {
    Preamble,
    Add,
    Delete,
    Update(Chunk),
    End,
}

fn invalid() -> io::Error {
    crate::usage("cannot read apply_patch hunks")
}

impl Hunk {
    fn finish(self) -> io::Result<()> {
        if matches!(
            self,
            Self::Update(Chunk::Start | Chunk::Moved | Chunk::NeedsLine)
        ) {
            Err(invalid())
        } else {
            Ok(())
        }
    }
}

impl Chunk {
    fn line<'a>(self, raw: &'a str, paths: &mut Vec<&'a str>) -> io::Result<Self> {
        let line = raw.trim_end();
        if self == Self::Start
            && let Some(path) = line.strip_prefix("*** Move to: ")
        {
            paths.push(path);
            return Ok(Self::Moved);
        }
        if line == "@@" || line.starts_with("@@ ") {
            return (self != Self::NeedsLine)
                .then_some(Self::NeedsLine)
                .ok_or_else(invalid);
        }
        if self == Self::End {
            return line.is_empty().then_some(self).ok_or_else(invalid);
        }
        if line == "*** End of File" {
            return match self {
                Self::NeedsLine => Err(invalid()),
                Self::Body => Ok(Self::End),
                other => Ok(other),
            };
        }
        if raw.is_empty() || raw.starts_with([' ', '+', '-']) {
            Ok(Self::Body)
        } else {
            Err(invalid())
        }
    }
}

// Match Codex df975851bf906d71e5a1ead04f1eaec41d26ca16 so context lines never become hook targets.
pub(super) fn targets(patch: &str) -> io::Result<Vec<&str>> {
    let mut lines = patch.trim().lines();
    let mut first = lines.next().ok_or_else(invalid)?;
    let mut last = lines.next_back().ok_or_else(invalid)?;
    if matches!(first, "<<EOF" | "<<'EOF'" | "<<\"EOF\"") && last.ends_with("EOF") {
        first = lines.next().ok_or_else(invalid)?;
        last = lines.next_back().ok_or_else(invalid)?;
    }
    if first.trim() != "*** Begin Patch" || last.trim() != END {
        return Err(invalid());
    }

    let mut paths = Vec::new();
    let mut state = Hunk::Preamble;
    let mut environment = false;
    for raw in lines {
        let raw = raw.strip_suffix('\r').unwrap_or(raw);
        let header = if matches!(state, Hunk::Update(_)) {
            raw.trim_end()
        } else {
            raw.trim()
        };
        if matches!(state, Hunk::End) {
            if header.is_empty() {
                continue;
            }
            return Err(invalid());
        }
        if matches!(state, Hunk::Preamble)
            && let Some(id) = header.strip_prefix("*** Environment ID:")
        {
            if environment || id.trim().is_empty() {
                return Err(invalid());
            }
            environment = true;
            continue;
        }
        if header == END {
            state.finish()?;
            state = Hunk::End;
            continue;
        }
        let next = [
            ("*** Add File: ", Hunk::Add),
            ("*** Delete File: ", Hunk::Delete),
            ("*** Update File: ", Hunk::Update(Chunk::Start)),
        ]
        .into_iter()
        .find_map(|(prefix, next)| header.strip_prefix(prefix).map(|p| (p, next)));
        if let Some((path, next)) = next {
            state.finish()?;
            paths.push(path);
            state = next;
        } else {
            state = match state {
                Hunk::Add if raw.starts_with('+') => Hunk::Add,
                Hunk::Update(chunk) => Hunk::Update(chunk.line(raw, &mut paths)?),
                _ => return Err(invalid()),
            };
        }
    }
    state.finish()?;
    Ok(paths)
}
