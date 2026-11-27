use std::{
    io::{self, Read, Write},
    process::{self, Child, Command, Output, Stdio},
    sync::Mutex,
    thread,
    time::{Duration, Instant},
};

// Leave time to block before Claude Code and Codex cancel their command hooks at 600 seconds.
const LIMIT: Duration = Duration::from_secs(300);

struct Run {
    activity: String,
    git: Option<(Child, String)>,
}

static RUN: Mutex<Option<Run>> = Mutex::new(None);

#[doc(hidden)]
pub struct Deadline;

/// Arms the command's wall-clock deadline, which exits with the given red status.
///
/// # Errors
/// Returns an error if the system cannot start the watchdog thread.
#[doc(hidden)]
pub fn start(code: i32, activity: &str) -> io::Result<Deadline> {
    let limit = std::env::var("BLACKHOLE_TEST_DEADLINE_MS")
        .ok()
        .and_then(|ms| ms.parse().ok())
        .map_or(LIMIT, |ms| LIMIT.min(Duration::from_millis(ms)));
    let end = Instant::now() + limit;
    *RUN.lock().unwrap() = Some(Run {
        activity: activity.into(),
        git: None,
    });
    thread::Builder::new().spawn(move || {
        thread::sleep(end.saturating_duration_since(Instant::now()));
        let mut run = RUN.lock().unwrap();
        if let Some(run) = run.as_mut() {
            let activity = match &mut run.git {
                Some((child, activity)) => {
                    let _ = child.kill();
                    let _ = child.try_wait();
                    activity.as_str()
                }
                None => &run.activity,
            };
            let _ = writeln!(
                io::stderr(),
                "blackhole: deadline exceeded while {activity}"
            );
            process::exit(code);
        }
    })?;
    Ok(Deadline)
}

impl Drop for Deadline {
    fn drop(&mut self) {
        *RUN.lock().unwrap() = None;
    }
}

pub(crate) fn activity(description: impl FnOnce() -> String) {
    if let Some(run) = RUN.lock().unwrap().as_mut() {
        run.activity = description();
    }
}

pub(crate) fn output(command: &mut Command) -> io::Result<Output> {
    let mut state = RUN.lock().unwrap();
    let Some(run) = state.as_mut() else {
        drop(state);
        return command.output();
    };
    let subcommand = command
        .get_args()
        .find(|arg| !arg.as_encoded_bytes().starts_with(b"-") && *arg != "core.fsmonitor=false")
        .map_or_else(String::new, |arg| format!(" {}", arg.to_string_lossy()));
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let mut stdout = child.stdout.take().unwrap();
    let mut stderr = child.stderr.take().unwrap();
    run.git = Some((child, format!("running git{subcommand}")));
    drop(state);
    let pipes = thread::scope(|scope| {
        let errors = thread::Builder::new().spawn_scoped(scope, move || drain(&mut stderr))?;
        let stdout = drain(&mut stdout);
        Ok::<_, io::Error>((stdout, errors.join().unwrap()))
    });
    let status = loop {
        let mut state = RUN.lock().unwrap();
        let run = state.as_mut().unwrap();
        let (child, _) = run.git.as_mut().unwrap();
        if pipes.is_err() {
            let _ = child.kill();
        }
        if let Some(status) = child.try_wait()? {
            run.git = None;
            break status;
        }
        drop(state);
        thread::sleep(Duration::from_millis(1));
    };
    let (stdout, stderr) = pipes?;
    Ok(Output {
        status,
        stdout: stdout?,
        stderr: stderr?,
    })
}

fn drain(stream: &mut impl Read) -> io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    stream.read_to_end(&mut bytes)?;
    Ok(bytes)
}
