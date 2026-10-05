//! The child process that unpacks an archive (`docs/specs/subtitles.md`, 압축 해제의
//! 격리와 한도, the row for the child process).
//!
//! It is started by [`crate::run::Unpacker`] with the arguments
//! `<out> <name> <part 1> [<part 2> ...]`, puts the limits of the process on
//! itself, runs [`crate::extract`] with [`Limits::default`] and answers with
//! one JSON line on its standard output: [`Answer`]. It also makes itself the
//! process the kernel's OOM killer picks, so that a container whose memory
//! limit it shares with the worker loses the child and not the worker.

use std::{
    ffi::OsString,
    io::Write,
    panic::{catch_unwind, AssertUnwindSafe},
    path::PathBuf,
    process::ExitCode,
};

use rustix::process::{Pid, Resource, Rlimit, Signal};
use serde::{Deserialize, Serialize};

use crate::{ExtractError, Limits, Member, Refusal};

/// The environment variable that holds the pid of the process that started
/// the child.
pub const PARENT_ENV: &str = "TRSS_EXTRACT_PARENT";

/// The exit code for arguments the child cannot use.
pub const EXIT_USAGE: u8 = 64;

/// `RLIMIT_AS`: the address space of the child.
pub(crate) const ADDRESS_SPACE: u64 = 256 << 20;
/// `RLIMIT_FSIZE`: the biggest file it may write, a little over the member
/// limit, which is what stops a member first.
const FILE_SIZE: u64 = 201 << 20;
/// `RLIMIT_NOFILE`.
const OPEN_FILES: u64 = 64;
/// `RLIMIT_CORE`: no core dump, which a memory abort, a SIGSEGV or a SIGXFSZ
/// would otherwise write (up to the address space of the child).
const CORE_SIZE: u64 = 0;
/// The `oom_score_adj` the child sets on itself: the highest, so the kernel's
/// OOM killer takes it before the worker or anything else of the container.
const OOM_SCORE_ADJ: &str = "1000";

/// What the child answers: the line it prints.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Answer {
    #[serde(rename = "done")]
    Done(Vec<Member>),
    #[serde(rename = "refused")]
    Refused(Refusal),
    /// The machine failed the unpacking, not the archive ([`ExtractError::Failed`]).
    #[serde(rename = "failed")]
    Failed(String),
}

/// Puts a hard limit on the process.
fn limit(resource: Resource, value: u64) -> std::io::Result<()> {
    rustix::process::setrlimit(
        resource,
        Rlimit {
            current: Some(value),
            maximum: Some(value),
        },
    )?;
    Ok(())
}

/// Makes the process the one the OOM killer picks first. Raising the value
/// needs no privilege (lowering it would).
fn prefer_oom_victim() -> std::io::Result<()> {
    std::fs::write("/proc/self/oom_score_adj", OOM_SCORE_ADJ)
}

/// Dies with the parent: the kernel sends SIGKILL when the parent thread ends,
/// and a parent that has gone already is noticed here. Returns whether to go on.
fn tied_to_parent() -> bool {
    if rustix::process::set_parent_process_death_signal(Some(Signal::KILL)).is_err() {
        return false;
    }
    let Some(expected) = std::env::var(PARENT_ENV)
        .ok()
        .and_then(|pid| pid.parse::<i32>().ok())
        .and_then(Pid::from_raw)
    else {
        // Started by hand, not by an `Unpacker`: nothing to compare.
        return true;
    };
    rustix::process::getppid() == Some(expected)
}

/// The child process's main: `<program> <out> <name> <part 1> [<part 2> ...]`.
///
/// Reads the pid of the parent from the environment (`TRSS_EXTRACT_PARENT`),
/// asks to be killed with it and exits at once if it is gone already; writes
/// `1000` to `/proc/self/oom_score_adj`, so that the kernel's OOM killer picks
/// this process and not the worker when the container's memory runs out;
/// sets `RLIMIT_AS` 256 MiB, `RLIMIT_FSIZE` 201 MiB, `RLIMIT_NOFILE` 64 and
/// `RLIMIT_CORE` 0 (no core dumps); runs [`crate::extract`] with
/// [`Limits::default`]; and prints one JSON line, `{"done":[...]}`,
/// `{"refused":...}` or `{"failed":"..."}` (the machine failed it, not the
/// archive). A panic of a decoder is a [`Refusal::Corrupt`]. The setting of
/// the OOM score and each limit fails closed: when one cannot be set, the
/// process says so on its standard error and exits with failure before it
/// reads the archive. Single-threaded. Exits with 64 when the arguments are
/// wrong.
pub fn main() -> ExitCode {
    let args: Vec<OsString> = std::env::args_os().skip(1).collect();
    if args.len() < 3 {
        eprintln!("usage: trss-extract <out> <name> <part 1> [<part 2> ...]");
        return ExitCode::from(EXIT_USAGE);
    }
    if !tied_to_parent() {
        return ExitCode::FAILURE;
    }
    if let Err(error) = prefer_oom_victim() {
        eprintln!("cannot raise the OOM score of the process: {error}");
        return ExitCode::FAILURE;
    }
    for (resource, value) in [
        (Resource::As, ADDRESS_SPACE),
        (Resource::Fsize, FILE_SIZE),
        (Resource::Nofile, OPEN_FILES),
        (Resource::Core, CORE_SIZE),
    ] {
        if let Err(error) = limit(resource, value) {
            eprintln!("cannot set a limit of the process: {error}");
            return ExitCode::FAILURE;
        }
    }

    let out = PathBuf::from(&args[0]);
    let name = args[1].to_string_lossy().into_owned();
    let parts: Vec<PathBuf> = args[2..].iter().map(PathBuf::from).collect();
    let result = catch_unwind(AssertUnwindSafe(|| {
        crate::extract(&parts, &name, &out, &Limits::default())
    }));
    let answer = match result {
        Ok(Ok(members)) => Answer::Done(members),
        Ok(Err(ExtractError::Refused(refusal))) => Answer::Refused(refusal),
        Ok(Err(ExtractError::Failed(message))) => Answer::Failed(message),
        Err(_) => Answer::Refused(Refusal::Corrupt {
            detail: "해석기 오류".to_owned(),
        }),
    };
    let mut line = match serde_json::to_vec(&answer) {
        Ok(line) => line,
        Err(error) => {
            eprintln!("cannot write the answer: {error}");
            return ExitCode::FAILURE;
        }
    };
    line.push(b'\n');
    let mut stdout = std::io::stdout().lock();
    if stdout
        .write_all(&line)
        .and_then(|()| stdout.flush())
        .is_err()
    {
        return ExitCode::FAILURE;
    }
    ExitCode::SUCCESS
}
