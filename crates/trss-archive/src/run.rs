//! Starting the child process that unpacks an archive and reading what it
//! answers (`docs/specs/subtitles.md`, 압축 해제의 격리와 한도).
//!
//! The parent gives the child a process group of its own, so that the time limit
//! and a cancellation end the child and everything it started, and kills the
//! group once more after the child is gone. What the child prints is not
//! trusted: it is validated before it is believed.

use std::{
    collections::HashSet,
    io::Read,
    os::unix::process::{CommandExt, ExitStatusExt},
    path::{Path, PathBuf},
    process::{Command, ExitStatus, Stdio},
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

use rustix::process::{kill_process_group, Pid, Signal};

use crate::{
    child::Answer, child::PARENT_ENV, model::MEMORY_LIMIT, name::check_path, Limits, Member,
    Refusal,
};

/// The time the whole unpacking may take, nested archives included.
pub const TIME_LIMIT: Duration = Duration::from_secs(120);

/// How much of the child's standard output is kept; the rest is read and dropped.
const STDOUT_CAP: usize = 16 << 20;
/// How much of its standard error is kept.
const STDERR_CAP: usize = 64 << 10;
/// How often the child is looked at.
const POLL: Duration = Duration::from_millis(20);
/// How much of a line of the child's standard error goes in a message.
const SHOWN: usize = 200;
/// The most bytes of the reason the child gives for a failure it believes: its
/// messages are far shorter (a prefix and 200 characters).
const FAILURE_CAP: usize = 2048;

/// The program that unpacks archives and the time it has.
#[derive(Debug, Clone)]
pub struct Unpacker {
    program: PathBuf,
    time_limit: Duration,
}

/// How an unpacking ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Unpacked {
    /// The members written to the output folder.
    Done(Vec<Member>),
    /// The archive was not unpacked, for a reason of its own.
    Refused(Refusal),
    /// The unpacking failed for a reason that is not the archive's: the child
    /// did not answer (it timed out, was killed, crashed or could not be
    /// started), or it answered that the machine failed it (a full disk, no
    /// memory). The Korean reason.
    Failed(String),
    /// `cancelled()` turned true and the child was killed.
    Cancelled,
}

impl Unpacker {
    /// An unpacker that runs `program` (the `trss-extract` binary), with the
    /// time limit [`TIME_LIMIT`].
    pub fn new(program: impl Into<PathBuf>) -> Unpacker {
        Unpacker {
            program: program.into(),
            time_limit: TIME_LIMIT,
        }
    }

    pub fn with_time_limit(mut self, limit: Duration) -> Unpacker {
        self.time_limit = limit;
        self
    }

    pub fn program(&self) -> &Path {
        &self.program
    }

    /// Unpacks the archive whose volumes are `parts` into the folder `out`,
    /// in a child process.
    ///
    /// Blocking: the calling thread stays here until the child is gone. That
    /// matters, because the child asks the kernel to kill it when the thread
    /// that started it ends (`PR_SET_PDEATHSIG`), so a caller must not start
    /// it from a thread that may exit early; waiting here keeps the thread.
    ///
    /// `out` is removed if it exists and its parent folders are created. The
    /// child runs with an empty environment but its parent's pid, in a
    /// process group of its own, with no standard input; its output and
    /// error are read by two threads, which keep the first 16 MiB and 64 KiB.
    /// Past the time limit, or once `cancelled()` is true, the whole group is
    /// killed. After the child ends, the group is killed again, so that
    /// nothing it started is left. `out` stays only when the answer is
    /// [`Unpacked::Done`].
    pub fn run(
        &self,
        parts: &[PathBuf],
        name: &str,
        out: &Path,
        cancelled: &dyn Fn() -> bool,
    ) -> Unpacked {
        remove_folder(out);
        if let Some(parent) = out.parent() {
            if let Err(error) = std::fs::create_dir_all(parent) {
                return Unpacked::Failed(spawn_failed(&error));
            }
        }
        if cancelled() {
            return Unpacked::Cancelled;
        }
        let spawned = Command::new(&self.program)
            .arg(out)
            .arg(name)
            .args(parts)
            .env_clear()
            .env(PARENT_ENV, std::process::id().to_string())
            .process_group(0)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn();
        let mut child = match spawned {
            Ok(child) => child,
            Err(error) => return Unpacked::Failed(spawn_failed(&error)),
        };
        let group = Pid::from_child(&child);
        let stdout = drain(child.stdout.take(), STDOUT_CAP);
        let stderr = drain(child.stderr.take(), STDERR_CAP);

        let started = Instant::now();
        let outcome = loop {
            match child.try_wait() {
                Ok(Some(status)) => break Outcome::Exited(status),
                Ok(None) => {}
                Err(error) => break Outcome::Lost(error.to_string()),
            }
            if cancelled() {
                break Outcome::Cancelled;
            }
            if started.elapsed() >= self.time_limit {
                break Outcome::TimedOut;
            }
            thread::sleep(POLL);
        };
        if !matches!(outcome, Outcome::Exited(_)) {
            let _ = kill_process_group(group, Signal::KILL);
            let _ = child.wait();
        }
        // Whatever the child started is gone with it. No such group is fine.
        let _ = kill_process_group(group, Signal::KILL);
        let stdout = stdout.join().unwrap_or_default();
        let stderr = stderr.join().unwrap_or_default();

        let unpacked = match outcome {
            Outcome::Cancelled => Unpacked::Cancelled,
            Outcome::TimedOut => Unpacked::Failed(format!(
                "{}초 안에 다 풀지 못해 멈췄어요",
                self.time_limit.as_millis().div_ceil(1000)
            )),
            Outcome::Lost(error) => Unpacked::Failed(format!(
                "압축을 풀던 프로세스가 오류로 끝났어요: {}",
                cut(&error)
            )),
            Outcome::Exited(status) => judge(status, stdout, &stderr),
        };
        if !matches!(unpacked, Unpacked::Done(_)) {
            remove_folder(out);
        }
        unpacked
    }
}

enum Outcome {
    Exited(ExitStatus),
    TimedOut,
    Cancelled,
    /// Waiting for the child failed.
    Lost(String),
}

/// Removes `out`, a folder, or whatever else is there.
fn remove_folder(out: &Path) {
    match std::fs::symlink_metadata(out) {
        Ok(meta) if meta.is_dir() => {
            let _ = std::fs::remove_dir_all(out);
        }
        Ok(_) => {
            let _ = std::fs::remove_file(out);
        }
        Err(_) => {}
    }
}

fn spawn_failed(error: &std::io::Error) -> String {
    format!("압축을 풀 프로세스를 띄우지 못했어요: {error}")
}

/// The first `SHOWN` characters of `text`.
fn cut(text: &str) -> String {
    text.chars().take(SHOWN).collect()
}

/// What a pipe held: the first bytes, and the rest read and dropped.
#[derive(Default)]
struct Captured {
    bytes: Vec<u8>,
    overflowed: bool,
}

/// Reads `pipe` to its end on a thread of its own, keeping the first `cap` bytes.
fn drain(pipe: Option<impl Read + Send + 'static>, cap: usize) -> JoinHandle<Captured> {
    thread::spawn(move || {
        let mut captured = Captured::default();
        let Some(mut pipe) = pipe else {
            return captured;
        };
        let mut chunk = [0u8; 8192];
        loop {
            match pipe.read(&mut chunk) {
                Ok(0) => break,
                Ok(n) => {
                    let room = cap.saturating_sub(captured.bytes.len());
                    captured.bytes.extend_from_slice(&chunk[..n.min(room)]);
                    if n > room {
                        captured.overflowed = true;
                    }
                }
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
                Err(_) => break,
            }
        }
        captured
    })
}

/// What the child's exit and output say.
fn judge(status: ExitStatus, stdout: Captured, stderr: &Captured) -> Unpacked {
    let stderr = String::from_utf8_lossy(&stderr.bytes);
    if let Some(signal) = status.signal() {
        return Unpacked::Failed(match signal {
            6 if stderr.contains("memory allocation of") => MEMORY_LIMIT.to_owned(),
            25 => "압축을 풀다가 파일 크기 한도에 닿았어요".to_owned(),
            other => format!("압축을 풀던 프로세스가 {}로 끝났어요", signal_name(other)),
        });
    }
    if !status.success() {
        let line = stderr.lines().next().unwrap_or("");
        return Unpacked::Failed(format!(
            "압축을 풀던 프로세스가 오류로 끝났어요: {}",
            cut(line)
        ));
    }
    match read_answer(&stdout) {
        Some(Answer::Done(members)) => Unpacked::Done(members),
        Some(Answer::Refused(refusal)) => Unpacked::Refused(refusal),
        Some(Answer::Failed(message)) => Unpacked::Failed(message),
        None => Unpacked::Failed("압축을 풀던 프로세스의 답을 읽지 못했어요".to_owned()),
    }
}

fn signal_name(signal: i32) -> String {
    match signal {
        1 => "SIGHUP",
        2 => "SIGINT",
        3 => "SIGQUIT",
        4 => "SIGILL",
        6 => "SIGABRT",
        7 => "SIGBUS",
        8 => "SIGFPE",
        9 => "SIGKILL",
        11 => "SIGSEGV",
        13 => "SIGPIPE",
        15 => "SIGTERM",
        24 => "SIGXCPU",
        25 => "SIGXFSZ",
        other => return format!("시그널 {other}"),
    }
    .to_owned()
}

/// The child's answer, if its standard output is one valid line.
fn read_answer(stdout: &Captured) -> Option<Answer> {
    if stdout.overflowed {
        return None;
    }
    let text = std::str::from_utf8(&stdout.bytes).ok()?;
    let line = text.strip_suffix('\n')?;
    if line.contains('\n') {
        return None;
    }
    let answer: Answer = serde_json::from_str(line).ok()?;
    let sound = match &answer {
        Answer::Done(members) => members_are_sound(members),
        Answer::Refused(_) => true,
        Answer::Failed(message) => failure_is_sound(message),
    };
    sound.then_some(answer)
}

/// Whether the reason the child gives for a failure could be one it writes: a
/// line of text, not empty and of a length a message carries.
fn failure_is_sound(message: &str) -> bool {
    !message.is_empty() && message.len() <= FAILURE_CAP && !message.chars().any(char::is_control)
}

/// Whether the members the child lists could be what an unpacking writes:
/// each `file` a plain decimal index no other has, each `sha256` 64 lowercase
/// hex digits, each `path` one a member may have.
fn members_are_sound(members: &[Member]) -> bool {
    let limits = Limits::default();
    let mut files = HashSet::new();
    members.iter().all(|member| {
        let index = !member.file.is_empty()
            && member.file.len() <= 20
            && member.file.bytes().all(|b| b.is_ascii_digit())
            && (member.file == "0" || !member.file.starts_with('0'));
        let digest = member.sha256.len() == 64
            && member
                .sha256
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b));
        index && digest && check_path(&member.path, &limits).is_ok() && files.insert(&member.file)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn member(path: &str, file: &str, sha256: &str) -> Member {
        Member {
            path: path.to_owned(),
            file: file.to_owned(),
            size: 1,
            sha256: sha256.to_owned(),
        }
    }

    fn captured(text: &str) -> Captured {
        Captured {
            bytes: text.as_bytes().to_vec(),
            overflowed: false,
        }
    }

    #[test]
    fn a_sound_answer_is_read() {
        let hex = "a".repeat(64);
        let line = format!(
            "{{\"done\":[{{\"path\":\"a/b.ass\",\"file\":\"0\",\"size\":1,\"sha256\":\"{hex}\"}}]}}\n"
        );
        let Some(Answer::Done(members)) = read_answer(&captured(&line)) else {
            panic!("not read");
        };
        assert_eq!(members, vec![member("a/b.ass", "0", &hex)]);
        assert_eq!(
            read_answer(&captured("{\"refused\":\"Ratio\"}\n")),
            Some(Answer::Refused(Refusal::Ratio))
        );
        assert_eq!(
            read_answer(&captured("{\"failed\":\"쓰지 못했어요\"}\n")),
            Some(Answer::Failed("쓰지 못했어요".to_owned()))
        );
    }

    #[test]
    fn an_answer_that_is_not_one_valid_line_is_not_read() {
        for text in [
            "",
            "{\"refused\":\"Ratio\"}",
            "{\"refused\":\"Ratio\"}\n{\"refused\":\"Ratio\"}\n",
            "not json\n",
            "{\"refused\":\"Nothing\"}\n",
            // A failure with no reason, with a line break in it, with a
            // control character, and with a reason that is too long.
            "{\"failed\":\"\"}\n",
            "{\"failed\":\"a\\nb\"}\n",
            "{\"failed\":\"a\\u001b[31mb\"}\n",
            &format!("{{\"failed\":\"{}\"}}\n", "a".repeat(FAILURE_CAP + 1)),
        ] {
            assert_eq!(read_answer(&captured(text)), None, "{text:?}");
        }
        let overflowed = Captured {
            bytes: b"{\"refused\":\"Ratio\"}\n".to_vec(),
            overflowed: true,
        };
        assert_eq!(read_answer(&overflowed), None);
    }

    #[test]
    fn members_that_cannot_be_what_an_unpacking_writes_are_refused() {
        let hex = "0123456789abcdef".repeat(4);
        assert!(members_are_sound(&[
            member("a", "0", &hex),
            member("b/c", "12", &hex)
        ]));
        assert!(members_are_sound(&[]));
        for bad in [
            vec![member("a", "../x", &hex)],
            vec![member("a", "", &hex)],
            vec![member("a", "01", &hex)],
            vec![member("a", "-1", &hex)],
            vec![member("a", "0", &"A".repeat(64))],
            vec![member("a", "0", "abc")],
            vec![member("", "0", &hex)],
            vec![member("../a", "0", &hex)],
            vec![member("/a", "0", &hex)],
            vec![member("a", "0", &hex), member("b", "0", &hex)],
        ] {
            assert!(!members_are_sound(&bad), "{bad:?}");
        }
    }

    #[test]
    fn signals_have_names() {
        assert_eq!(signal_name(9), "SIGKILL");
        assert_eq!(signal_name(40), "시그널 40");
    }
}
