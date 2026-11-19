use globset::{Glob, GlobSet, GlobSetBuilder};
use std::{path::Path, sync::LazyLock};

static CONFIG_NAMES: LazyLock<GlobSet> = LazyLock::new(|| {
    let mut names = GlobSetBuilder::new();
    for pattern in [
        "{tsconfig,jsconfig}.json",
        "{tsconfig,jsconfig}.*.json",
        "package.json",
        "eslint-local-rules.*",
        ".simple-git-hooks.*",
        "simple-git-hooks.*",
        "{biome,deno}.{json,jsonc}",
        ".editorconfig",
        "{eslint,oxlint,stylelint,prettier,vitest,jest,vite,playwright,cypress,nyc,karma,lint-staged}.config.*",
        "vitest.workspace.*",
        ".{eslint,oxlint,oxfmt,prettier,stylelint,mocha,nyc,c8,lintstaged,husky}rc",
        ".{eslint,oxlint,oxfmt,prettier,stylelint,mocha,nyc,c8,lintstaged,husky}rc.*",
        ".{eslint,prettier,stylelint}ignore",
        ".coveragerc",
        "codecov.y*ml",
        ".codecov.y*ml",
        "lefthook.{yml,yaml,toml,json}",
        ".lefthook.{yml,yaml,toml,json}",
        ".pre-commit-config.y*ml",
        ".{clippy,rustfmt}.toml",
        "{clippy,rustfmt,ruff,pyproject,Cargo,rust-toolchain}.toml",
        "pytest.ini",
        "tox.ini",
        "setup.cfg",
        ".flake8",
        ".golangci.*",
    ] {
        names.add(Glob::new(pattern).expect("built-in judge filename"));
    }
    names.build().expect("built-in judge filenames")
});

pub(crate) fn protected(path: &str) -> bool {
    let mut parts = path.rsplit('/');
    let name = parts.next().unwrap_or_default();
    path == "blackhole.toml"
        || path == "bin/check"
        || Path::new(path).ends_with(".cargo/config.toml")
        || Path::new(path).ends_with(".cargo/config")
        || path.starts_with(".github/")
        || path.starts_with(".blackhole/")
        || path.starts_with("eslint/rules/")
        || path.contains("/eslint/rules/")
        || CONFIG_NAMES.is_match(name)
        || parts.any(|dir| {
            matches!(
                dir,
                ".githooks"
                    | ".husky"
                    | ".eslint-rules"
                    | "eslint-rules"
                    | "eslint-local-rules"
                    | "lint-rules"
            ) || dir.starts_with("eslint-plugin-")
        })
}
