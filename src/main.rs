use blackhole_check::{Fault, ci::State, hooks, init};
use clap::{Parser, Subcommand};
use std::{
    io::{self, Write},
    path::PathBuf,
    process::ExitCode,
};

#[derive(Parser)]
#[command(name = "blackhole", version)]
struct Args {
    #[command(subcommand)]
    command: Action,
}

#[derive(Subcommand)]
enum Action {
    Init,
    Hook {
        event: hooks::Event,
    },
    Ledger,
    Ci {
        #[arg(long)]
        base: String,
        #[arg(long)]
        head: String,
        #[arg(long)]
        approvals: Option<PathBuf>,
        /// Trusted external directory containing <head SHA>/<base-configured LCOV path>.
        #[arg(long)]
        coverage_dir: Option<PathBuf>,
        #[arg(long)]
        today: Option<blackhole_check::objections::Date>,
    },
    Object {
        #[arg(long)]
        renew: bool,
        rule: String,
        path: String,
        reason: Option<String>,
    },
    Check {
        paths: Vec<PathBuf>,
    },
}

// Exit 3: only a person's decision clears the verdict.
const WAITING: u8 = 3;
// Exit 75, EX_TEMPFAIL: the tool could not judge, so the caller retries or reports instead of reading a verdict.
const NO_VERDICT: u8 = 75;

fn main() -> ExitCode {
    let mut out = io::stdout().lock();
    let command = Args::parse().command;
    let ci = matches!(command, Action::Ci { .. });
    let result = match command {
        Action::Init => {
            return init::run(&mut out).map_or_else(
                |e| {
                    let _ = writeln!(io::stderr().lock(), "blackhole: {e}");
                    ExitCode::from(2)
                },
                |()| ExitCode::SUCCESS,
            );
        }
        Action::Hook { event } => return hooks::run(event),
        Action::Check { paths } => blackhole_check::check(&paths, &mut out).map(u8::from),
        Action::Object {
            rule,
            path,
            reason,
            renew,
        } => blackhole_check::object(rule, path, reason.as_deref(), renew).map(|()| 0),
        Action::Ci {
            base,
            head,
            approvals,
            today,
            coverage_dir,
        } => blackhole_check::ci::run_with_coverage(
            &base,
            &head,
            approvals.as_deref(),
            today,
            coverage_dir.as_deref(),
            &mut out,
        )
        .map(|state| match state {
            State::Green => 0,
            State::Red => 1,
            State::Waiting => WAITING,
        }),
        Action::Ledger => blackhole_check::regenerate(&mut out).map(u8::from),
    };
    let error = match result {
        Ok(code) => return ExitCode::from(code),
        Err(error) => error,
    };
    // A closed pipe must not change the exit code, which is the verdict.
    let code = match blackhole_check::fault(&error) {
        Some(Fault::Usage) => {
            let _ = writeln!(io::stderr().lock(), "blackhole: {error}");
            2
        }
        Some(Fault::Red) => {
            let _ = writeln!(out, "blackhole: {error}");
            1
        }
        None if ci => {
            let _ = writeln!(out, "blackhole ci: no verdict: {error}");
            NO_VERDICT
        }
        None => {
            let _ = writeln!(io::stderr().lock(), "blackhole: no verdict: {error}");
            NO_VERDICT
        }
    };
    ExitCode::from(code)
}
