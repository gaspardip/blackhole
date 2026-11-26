use crate::engine::{Family, Gate, Rule, Run, StructureCtx, source_type};
use std::path::Path;

pub const RULE: Rule = Rule {
    id: "ts/test-file-names",
    family: Family::Structure,
    gate: Gate::Blocking,
    provenance: "landmines: test-file-names",
    fix: "Use <subject>.test, <subject>.spec, <subject>.fixture or <subject>.test-utils before the extension. Move tests beside their implementation or into a test directory.",
    run: Run::Structure(check),
};

fn name_problem(stem: &str) -> Option<&'static str> {
    let suffix = stem.rsplit_once('.').map(|(_, suffix)| suffix);
    if matches!(suffix, Some("specs" | "tests")) {
        return Some("use <subject>.test");
    }
    if matches!(
        stem.rsplit(['-', '_', '.']).next(),
        Some("fixture" | "fixtures")
    ) && stem.strip_suffix(".fixture").is_none_or(str::is_empty)
    {
        return Some("use <subject>.fixture");
    }
    matches!(
        suffix,
        Some(
            "test-helper"
                | "test-helpers"
                | "test-util"
                | "testutil"
                | "testutils"
                | "test-utilities"
                | "helper"
                | "helpers"
                | "mock"
                | "mocks"
                | "stub"
                | "stubs"
                | "harness"
        )
    )
    .then_some("use <subject>.test-utils")
}

fn check(ctx: &StructureCtx<'_>, notes: &mut Vec<String>) {
    if source_type(ctx.path).is_none() {
        return;
    }
    let path = Path::new(ctx.path);
    let misnamed = path
        .file_name()
        .and_then(|name| name.to_str())
        .and_then(|name| name.rsplit_once('.'))
        .filter(|(_, ext)| super::script_extension(ext))
        .and_then(|(stem, _)| name_problem(stem));
    notes.extend(misnamed.map(str::to_owned));
    if misplaced_test_dir(ctx, path) {
        notes.push("use a test directory".into());
    }
}

fn misplaced_test_dir(ctx: &StructureCtx<'_>, path: &Path) -> bool {
    let mut misplaced = false;
    for dir in path.ancestors().skip(1) {
        ctx.record_work(1);
        if ctx.is_package(dir) {
            break;
        }
        misplaced |= dir
            .file_name()
            .is_some_and(|name| name == "tests" || name == "__tests__");
    }
    misplaced
}
