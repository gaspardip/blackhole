use std::process::{Command, Output};
use tempfile::tempdir;

const TOKEN: &str = "not-a-real-token";

fn judge(base: &str) -> Output {
    let dir = tempdir().unwrap();
    Command::new("bash")
        .arg(concat!(env!("CARGO_MANIFEST_DIR"), "/.github/judge.sh"))
        .current_dir(dir.path())
        .env("GH_TOKEN", TOKEN)
        .env("BASE_REF", base)
        .env("HEAD_SHA", "0")
        .env("NUMBER", "1")
        .env("GITHUB_REPOSITORY", "o/r")
        .env(
            "GITHUB_SERVER_URL",
            format!("file://{}", dir.path().display()),
        )
        .output()
        .unwrap()
}

#[test]
fn a_target_another_branch_could_share_a_check_name_with_is_refused() {
    for base in ["Main", "ma\u{ed}n", "release^1", "a b"] {
        let output = judge(base);
        let said = String::from_utf8(output.stdout).unwrap();
        assert!(
            !output.status.success() && said.contains("cannot be judged"),
            "{base}: {said}"
        );
    }
}

#[test]
fn an_allowed_target_reaches_the_fetch_and_the_token_stays_out_of_the_output() {
    let output = judge("release/1.2_x-y");
    let said = String::from_utf8(output.stdout).unwrap() + &String::from_utf8_lossy(&output.stderr);
    assert!(
        !output.status.success()
            && !said.contains("cannot be judged")
            && said.contains("o/r.git")
            && !said.contains(TOKEN),
        "{said}"
    );
}
