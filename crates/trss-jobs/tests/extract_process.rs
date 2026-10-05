//! The child process that unpacks an archive, run as the worker runs it
//! (`docs/specs/subtitles.md`, 압축 해제의 격리와 한도).

use std::{
    io::{Cursor, Write},
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    sync::Mutex,
    time::Duration,
};

use sha2::{Digest, Sha256};
use tempfile::TempDir;
use trss_archive::{
    run::{Unpacked, Unpacker},
    Refusal,
};

/// A script that is written and then run can fail to start while another
/// thread of the tests is starting a process ("text file busy"), so the tests
/// start their processes one at a time.
static START: Mutex<()> = Mutex::new(());

const EXTRACT: &str = env!("CARGO_BIN_EXE_trss-extract");

struct Case {
    dir: TempDir,
}

impl Case {
    fn new() -> Case {
        Case {
            dir: tempfile::tempdir().unwrap(),
        }
    }

    fn out(&self) -> PathBuf {
        self.dir.path().join("work/out")
    }

    fn write(&self, name: &str, bytes: &[u8]) -> PathBuf {
        let path = self.dir.path().join(name);
        std::fs::write(&path, bytes).unwrap();
        path
    }

    /// A named pipe nobody writes to: opening it for reading blocks.
    fn fifo(&self, name: &str) -> PathBuf {
        let path = self.dir.path().join(name);
        let made = std::process::Command::new("mkfifo")
            .arg(&path)
            .status()
            .unwrap();
        assert!(made.success());
        path
    }

    /// A program that is the shell script `body`.
    fn script(&self, name: &str, body: &str) -> PathBuf {
        let _start = START
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let path = self.dir.path().join(name);
        {
            let mut file = std::fs::File::create(&path).unwrap();
            file.write_all(format!("#!/bin/sh\n{body}\n").as_bytes())
                .unwrap();
            file.sync_all().unwrap();
        }
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path
    }

    fn run(&self, unpacker: &Unpacker, parts: &[PathBuf], name: &str) -> Unpacked {
        self.run_cancelled(unpacker, parts, name, &|| false)
    }

    fn run_cancelled(
        &self,
        unpacker: &Unpacker,
        parts: &[PathBuf],
        name: &str,
        cancelled: &dyn Fn() -> bool,
    ) -> Unpacked {
        let _start = START
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        unpacker.run(parts, name, &self.out(), cancelled)
    }

    fn assert_no_output(&self) {
        assert!(!self.out().exists(), "{:?} is left", self.out());
    }
}

fn zip_of(entries: &[(&str, &[u8])], deflate: bool) -> Vec<u8> {
    let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let method = if deflate {
        zip::CompressionMethod::Deflated
    } else {
        zip::CompressionMethod::Stored
    };
    let options = zip::write::SimpleFileOptions::default().compression_method(method);
    for (name, bytes) in entries {
        writer.start_file(*name, options).unwrap();
        writer.write_all(bytes).unwrap();
    }
    writer.finish().unwrap().into_inner()
}

fn hex_sha256(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// Whether the process `pid` is gone (or only waiting to be reaped).
fn is_gone(pid: u32) -> bool {
    match std::fs::read_to_string(format!("/proc/{pid}/stat")) {
        Err(_) => true,
        Ok(stat) => stat
            .rsplit(')')
            .next()
            .is_some_and(|rest| rest.trim_start().starts_with('Z')),
    }
}

fn wait_until_gone(pid: u32) -> bool {
    for _ in 0..100 {
        if is_gone(pid) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    false
}

fn read_pid(path: &Path) -> u32 {
    for _ in 0..100 {
        if let Ok(text) = std::fs::read_to_string(path) {
            if let Ok(pid) = text.trim().parse() {
                return pid;
            }
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    panic!("no pid in {path:?}");
}

#[test]
fn a_zip_is_unpacked_by_the_child() {
    let case = Case::new();
    let a = b"first subtitle".to_vec();
    let b = vec![7u8; 5000];
    let part = case.write(
        "pack.zip",
        &zip_of(&[("sub/a.ass", &a), ("b.srt", &b)], true),
    );
    let unpacker = Unpacker::new(EXTRACT);
    let Unpacked::Done(members) = case.run(&unpacker, &[part], "pack.zip") else {
        panic!("not unpacked");
    };
    assert_eq!(members.len(), 2);
    for (member, (path, bytes)) in members.iter().zip([("sub/a.ass", &a), ("b.srt", &b)]) {
        assert_eq!(member.path, path);
        assert_eq!(member.size, bytes.len() as u64);
        assert_eq!(member.sha256, hex_sha256(bytes));
        assert_eq!(
            &std::fs::read(case.out().join(&member.file)).unwrap(),
            bytes
        );
    }
    // Only the members are in the folder.
    assert_eq!(std::fs::read_dir(case.out()).unwrap().count(), 2);
}

#[test]
fn a_zip_bomb_is_refused_and_leaves_nothing() {
    let case = Case::new();
    let zeros = vec![0u8; 24 << 20];
    let part = case.write("bomb.zip", &zip_of(&[("zeros.bin", &zeros)], true));
    let unpacked = case.run(&Unpacker::new(EXTRACT), &[part], "bomb.zip");
    assert!(
        matches!(
            unpacked,
            Unpacked::Refused(Refusal::Ratio | Refusal::TooLarge { .. })
        ),
        "{unpacked:?}"
    );
    case.assert_no_output();
}

#[test]
fn a_refusal_of_the_child_is_passed_on() {
    let case = Case::new();
    let part = case.write("esc.zip", &zip_of(&[("../x.ass", b"x")], false));
    let unpacked = case.run(&Unpacker::new(EXTRACT), &[part], "esc.zip");
    assert!(
        matches!(unpacked, Unpacked::Refused(Refusal::Path { .. })),
        "{unpacked:?}"
    );
    case.assert_no_output();
}

#[test]
fn a_child_that_hangs_on_its_input_is_killed_at_the_time_limit() {
    let case = Case::new();
    let fifo = case.fifo("pack.zip");
    let unpacker = Unpacker::new(EXTRACT).with_time_limit(Duration::from_secs(1));
    let started = std::time::Instant::now();
    let unpacked = case.run(&unpacker, &[fifo], "pack.zip");
    let Unpacked::Failed(reason) = unpacked else {
        panic!("{unpacked:?}");
    };
    assert!(
        reason.contains("1초 안에 다 풀지 못해 멈췄어요"),
        "{reason}"
    );
    assert!(started.elapsed() < Duration::from_secs(10));
    case.assert_no_output();
}

#[test]
fn the_whole_process_group_is_killed_at_the_time_limit() {
    let case = Case::new();
    let pids = case.dir.path().join("pids");
    // The program starts a process of its own that outlives it, then waits.
    let program = case.script(
        "hang.sh",
        &format!("sleep 300 &\necho $! > '{}'\nwait", pids.display()),
    );
    let unpacker = Unpacker::new(&program).with_time_limit(Duration::from_secs(1));
    let unpacked = case.run(&unpacker, &[], "pack.zip");
    assert!(matches!(unpacked, Unpacked::Failed(_)), "{unpacked:?}");
    assert!(wait_until_gone(read_pid(&pids)), "the grandchild is alive");
}

#[test]
fn what_a_child_left_running_is_killed_when_it_ends() {
    let case = Case::new();
    let pids = case.dir.path().join("pids");
    let program = case.script(
        "leave.sh",
        &format!(
            "sleep 300 >/dev/null 2>&1 &\necho $! > '{}'\necho '{{\"done\":[]}}'",
            pids.display()
        ),
    );
    let unpacked = case.run(&Unpacker::new(&program), &[], "pack.zip");
    assert_eq!(unpacked, Unpacked::Done(Vec::new()));
    assert!(wait_until_gone(read_pid(&pids)), "the grandchild is alive");
}

#[test]
fn a_cancellation_kills_the_child_and_leaves_nothing() {
    let case = Case::new();
    let fifo = case.fifo("pack.zip");
    let calls = std::sync::atomic::AtomicUsize::new(0);
    // Not cancelled when the child starts, cancelled a moment after.
    let cancelled = || calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst) >= 10;
    let unpacked = case.run_cancelled(&Unpacker::new(EXTRACT), &[fifo], "pack.zip", &cancelled);
    assert_eq!(unpacked, Unpacked::Cancelled);
    case.assert_no_output();

    // Cancelled from the start, the child is not started.
    let part = case.write("pack2.zip", &zip_of(&[("a.ass", b"x")], false));
    let unpacked = case.run_cancelled(&Unpacker::new(EXTRACT), &[part], "pack2.zip", &|| true);
    assert_eq!(unpacked, Unpacked::Cancelled);
    case.assert_no_output();
}

#[test]
fn a_child_that_crashes_or_cannot_start_is_a_failure() {
    let case = Case::new();
    let part = case.write("pack.zip", &zip_of(&[("a.ass", b"x")], false));
    for program in ["/bin/false", "/nonexistent/trss-extract"] {
        let unpacked = case.run(
            &Unpacker::new(program),
            std::slice::from_ref(&part),
            "pack.zip",
        );
        assert!(
            matches!(unpacked, Unpacked::Failed(_)),
            "{program}: {unpacked:?}"
        );
        case.assert_no_output();
    }
    // Killed by a signal.
    let program = case.script("killed.sh", "kill -SEGV $$");
    let unpacked = case.run(
        &Unpacker::new(&program),
        std::slice::from_ref(&part),
        "pack.zip",
    );
    assert!(matches!(unpacked, Unpacked::Failed(_)), "{unpacked:?}");
    // It exits well but says nothing.
    let program = case.script("silent.sh", "exit 0");
    let unpacked = case.run(&Unpacker::new(&program), &[part], "pack.zip");
    assert!(matches!(unpacked, Unpacked::Failed(_)), "{unpacked:?}");
    case.assert_no_output();
}

#[test]
fn what_the_child_prints_is_not_believed() {
    let case = Case::new();
    let part = case.write("pack.zip", &zip_of(&[("a.ass", b"x")], false));
    let digest = hex_sha256(b"x");
    let answers = [
        // A path out of the folder.
        format!(r#"{{"done":[{{"path":"../x","file":"0","size":1,"sha256":"{digest}"}}]}}"#),
        // A file name that is a path.
        format!(r#"{{"done":[{{"path":"a.ass","file":"../0","size":1,"sha256":"{digest}"}}]}}"#),
        format!(
            r#"{{"done":[{{"path":"a.ass","file":"/etc/passwd","size":1,"sha256":"{digest}"}}]}}"#
        ),
        // A digest that is not one.
        r#"{"done":[{"path":"a.ass","file":"0","size":1,"sha256":"zz"}]}"#.to_owned(),
        // Not what the child says.
        "not json".to_owned(),
        String::new(),
    ];
    for (index, answer) in answers.iter().enumerate() {
        let program = case.script(
            &format!("liar{index}.sh"),
            &format!("cat <<'END'\n{answer}\nEND"),
        );
        let unpacked = case.run(
            &Unpacker::new(&program),
            std::slice::from_ref(&part),
            "pack.zip",
        );
        let Unpacked::Failed(reason) = unpacked else {
            panic!("{answer}: {unpacked:?}");
        };
        // Not that the program could not be started.
        assert!(reason.contains("답을 읽지 못했어요"), "{answer}: {reason}");
        case.assert_no_output();
    }
}

#[test]
fn an_output_folder_that_is_there_is_replaced() {
    let case = Case::new();
    std::fs::create_dir_all(case.out()).unwrap();
    std::fs::write(case.out().join("old"), b"left from before").unwrap();
    let part = case.write("pack.zip", &zip_of(&[("a.ass", b"new")], false));
    let Unpacked::Done(members) = case.run(&Unpacker::new(EXTRACT), &[part], "pack.zip") else {
        panic!("not unpacked");
    };
    assert_eq!(members.len(), 1);
    assert!(!case.out().join("old").exists());
}

#[test]
fn a_failure_the_child_reports_is_a_failure_and_not_a_refusal() {
    let case = Case::new();
    let part = case.write("pack.zip", &zip_of(&[("a.ass", b"x")], false));
    let reason = "압축을 풀 자리에 쓰지 못했어요: No space left on device (os error 28)";
    let program = case.script("full.sh", &format!("echo '{{\"failed\":\"{reason}\"}}'"));
    let unpacked = case.run(&Unpacker::new(&program), &[part], "pack.zip");
    assert_eq!(unpacked, Unpacked::Failed(reason.to_owned()));
    case.assert_no_output();
}

#[test]
fn a_failure_the_child_reports_is_believed_only_when_it_could_be_one() {
    let case = Case::new();
    let part = case.write("pack.zip", &zip_of(&[("a.ass", b"x")], false));
    let answers = [
        // No reason, a line break and a control character in it, too long.
        r#"{"failed":""}"#.to_owned(),
        r#"{"failed":"a\nb"}"#.to_owned(),
        r#"{"failed":"\u001b[31mred"}"#.to_owned(),
        format!(r#"{{"failed":"{}"}}"#, "가".repeat(1000)),
        // Not a string.
        r#"{"failed":42}"#.to_owned(),
    ];
    for (index, answer) in answers.iter().enumerate() {
        let program = case.script(
            &format!("full{index}.sh"),
            &format!("cat <<'END'\n{answer}\nEND"),
        );
        let unpacked = case.run(
            &Unpacker::new(&program),
            std::slice::from_ref(&part),
            "pack.zip",
        );
        let Unpacked::Failed(reason) = unpacked else {
            panic!("{answer}: {unpacked:?}");
        };
        assert!(reason.contains("답을 읽지 못했어요"), "{answer}: {reason}");
        case.assert_no_output();
    }
}

#[test]
fn a_folder_the_child_cannot_write_in_is_a_failure_and_not_a_broken_archive() {
    let case = Case::new();
    let part = case.write("pack.zip", &zip_of(&[("a.ass", b"x")], false));
    // The unpacker makes the parent of `out`; the child cannot make `out` in
    // it. Nothing stops a user that may write anywhere, such as root.
    let work = case.dir.path().join("work");
    std::fs::create_dir_all(&work).unwrap();
    std::fs::set_permissions(&work, std::fs::Permissions::from_mode(0o555)).unwrap();
    let writable = std::fs::File::create(work.join("probe")).is_ok();
    let unpacked = case.run(&Unpacker::new(EXTRACT), &[part], "pack.zip");
    std::fs::set_permissions(&work, std::fs::Permissions::from_mode(0o755)).unwrap();
    if writable {
        return;
    }
    let Unpacked::Failed(reason) = unpacked else {
        panic!("{unpacked:?}");
    };
    assert!(
        reason.starts_with("압축을 풀 폴더를 만들지 못했어요"),
        "{reason}"
    );
    case.assert_no_output();
}

/// The value a line of `/proc/<pid>/limits` that begins with `name` has in its
/// soft limit column.
fn soft_limit(pid: u32, name: &str) -> Option<String> {
    let limits = std::fs::read_to_string(format!("/proc/{pid}/limits")).ok()?;
    let line = limits.lines().find(|line| line.starts_with(name))?;
    line[name.len()..]
        .split_whitespace()
        .next()
        .map(str::to_owned)
}

#[test]
fn the_child_is_the_process_the_oom_killer_picks_and_leaves_no_core_dump() {
    let case = Case::new();
    // The child blocks on opening the input, after it has put its limits on.
    let fifo = case.fifo("pack.zip");
    let mut child = std::process::Command::new(EXTRACT)
        .arg(case.out())
        .arg("pack.zip")
        .arg(&fifo)
        .env_clear()
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .unwrap();
    let pid = child.id();
    let read = |pid: u32| {
        (
            std::fs::read_to_string(format!("/proc/{pid}/oom_score_adj"))
                .map(|text| text.trim().to_owned())
                .ok(),
            soft_limit(pid, "Max core file size"),
        )
    };
    let mut seen = read(pid);
    for _ in 0..250 {
        if seen == (Some("1000".to_owned()), Some("0".to_owned())) {
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
        seen = read(pid);
    }
    let _ = child.kill();
    let _ = child.wait();
    assert_eq!(seen, (Some("1000".to_owned()), Some("0".to_owned())));
}

#[test]
fn a_zip_whose_end_record_is_far_from_its_end_is_refused_before_its_directory_is_read() {
    // 500,000 members and 70 KiB of junk: the crate would find the end record
    // anyway and read a directory that does not fit in the memory of the
    // child before it looked at the count.
    let case = Case::new();
    let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let options =
        zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    for index in 0..500_000 {
        writer.start_file(format!("f{index}.ass"), options).unwrap();
    }
    let mut bytes = writer.finish().unwrap().into_inner();
    bytes.extend_from_slice(&vec![b'x'; 70 << 10]);
    let part = case.write("many.zip", &bytes);
    drop(bytes);
    let unpacked = case.run(&Unpacker::new(EXTRACT), &[part], "many.zip");
    let Unpacked::Refused(Refusal::Corrupt { detail }) = unpacked else {
        panic!("{unpacked:?}");
    };
    assert!(detail.contains("ZIP의 끝을 찾지 못했어요"), "{detail}");
    case.assert_no_output();
}
