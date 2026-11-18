pub mod engine;
mod packs {
    pub(crate) mod core {
        pub(crate) mod no_suppression;
    }
}

mod files;

use std::{
    fs,
    io::{self, Write},
    path::PathBuf,
};

/// Checks the selected files and returns whether any rule was violated.
///
/// # Errors
/// Returns an error if discovery, file reading, or output fails.
pub fn check(paths: &[PathBuf], output: &mut impl Write) -> io::Result<bool> {
    let (root, names) = files::list(paths)?;
    let rules = engine::registry();
    let mut findings = Vec::new();
    for name in names {
        let bytes = fs::read(root.join(&name))
            .map_err(|e| io::Error::new(e.kind(), format!("{name:?}: {e}")))?;
        findings.extend(engine::run(rules, [(name.as_str(), bytes.as_slice())]));
    }
    for f in &findings {
        write!(output, "{}:{}:{} {}", f.path, f.line, f.col, f.rule)?;
        if let Some(note) = &f.note {
            write!(output, " {note}")?;
        }
        writeln!(output)?;
    }
    for rule in rules {
        if findings.iter().any(|f| f.rule == rule.id) {
            writeln!(output, "{} {}", rule.id, rule.fix)?;
        }
    }
    Ok(!findings.is_empty())
}
