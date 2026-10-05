//! The worker's own unpacking of a received archive, measured under the
//! container's memory limit (ticket 0065): `trss-extract` is started through
//! `trss_archive::run::Unpacker` as the worker does, and the container's
//! memory cgroup, which covers the child, is sampled meanwhile.

use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    time::Instant,
};

use trss_archive::run::{Unpacked, Unpacker};

use crate::{
    cgroup::Memcg,
    files::{self, Cleanup},
    memtest::{mib, peak_text},
    report::Report,
};

/// The name of an archive for the report: the file name of its first part.
fn archive_name(parts: &[PathBuf]) -> String {
    parts
        .first()
        .and_then(|part| part.file_name())
        .map_or_else(String::new, |name| name.to_string_lossy().into_owned())
}

/// Exact bytes beside MiB: a fixture is a few KiB.
fn size_text(bytes: u64) -> String {
    format!("{bytes} bytes, {}", mib(bytes))
}

fn count(events: &BTreeMap<String, u64>, key: &str) -> u64 {
    events.get(key).copied().unwrap_or(0)
}

/// Unpacks each archive of `archives` (the volumes of one, in order) into a
/// test folder under `data`, the app data disk where the worker's receive area
/// is, and reports what each took and the memory it used.
pub fn run(
    r: &mut Report,
    data: &Path,
    archives: &[Vec<PathBuf>],
    program: &Path,
    cleanup: &mut Cleanup,
) {
    if !program.is_file() {
        r.check(
            false,
            "unpack: the extract program",
            format!(
                "not found at {} (pass --extract; by default it is beside the probe)",
                program.display()
            ),
        );
        return;
    }
    r.info(format!("extract program {}", program.display()));

    let folder = data.join(format!("{}-unpack", files::unique_name()));
    cleanup.track(&folder);
    let made = fs::create_dir(&folder);
    r.check(
        made.is_ok(),
        "unpack: make the test folder",
        match &made {
            Ok(()) => folder.display().to_string(),
            Err(e) => format!("{}: {e}", folder.display()),
        },
    );
    if made.is_err() {
        return;
    }

    let memcg = Memcg::open();
    match &memcg {
        Some(memcg) => r.info(format!(
            "memory.max {}, memory.current {}",
            memcg.text("memory.max"),
            memcg.text("memory.current")
        )),
        None => r.info(
            "no cgroup v2 memory files under /sys/fs/cgroup: the unpacking runs but its memory is not measured",
        ),
    }

    let unpacker = Unpacker::new(program);
    for (n, parts) in archives.iter().enumerate() {
        let out = folder.join(format!("out-{n}"));
        measure(r, memcg.as_ref(), &unpacker, parts, &out);
        // What the unpacking left is not needed by the next one.
        let _ = fs::remove_dir_all(&out);
    }
}

/// One unpacking, as the worker runs it, and its report.
fn measure(
    r: &mut Report,
    memcg: Option<&Memcg>,
    unpacker: &Unpacker,
    parts: &[PathBuf],
    out: &Path,
) -> Option<Unpacked> {
    let name = archive_name(parts);
    let names = parts
        .iter()
        .map(|part| part.display().to_string())
        .collect::<Vec<_>>()
        .join(", ");
    let sizes: Result<Vec<u64>, String> = parts
        .iter()
        .map(|part| {
            fs::metadata(part)
                .map(|meta| meta.len())
                .map_err(|e| format!("{}: {e}", part.display()))
        })
        .collect();
    let input = match sizes {
        Ok(sizes) => sizes.iter().sum::<u64>(),
        Err(e) => {
            r.check(false, &format!("unpack {name}: the input"), e);
            return None;
        }
    };
    r.info(format!("unpack {name}: {names} ({})", size_text(input)));

    let events_before = memcg.map(|m| m.keyed("memory.events"));
    let sampler = memcg.map(Memcg::sample);
    let started = Instant::now();
    let unpacked = unpacker.run(parts, &name, out, &|| false);
    let elapsed = started.elapsed();
    let peak = sampler.map(|s| s.stop());

    let outcome = match &unpacked {
        Unpacked::Done(members) => format!(
            "done, {} members, {} unpacked",
            members.len(),
            size_text(members.iter().map(|m| m.size).sum())
        ),
        Unpacked::Refused(refusal) => format!("refused: {refusal}"),
        Unpacked::Failed(reason) => format!("failed: {reason}"),
        Unpacked::Cancelled => "cancelled".to_owned(),
    };
    r.info(format!(
        "unpack {name}: {outcome} in {:.1} s",
        elapsed.as_secs_f64()
    ));

    if let (Some(memcg), Some(peak), Some(before)) = (memcg, peak, events_before) {
        r.info(format!("unpack {name}: {}", peak_text(&peak)));
        let after = memcg.keyed("memory.events");
        let kills = count(&after, "oom_kill").saturating_sub(count(&before, "oom_kill"));
        r.check(
            kills == 0,
            &format!("unpack {name}: oom_kill during the unpack"),
            format!(
                "{} -> {} ({kills} new)",
                count(&before, "oom_kill"),
                count(&after, "oom_kill")
            ),
        );
        let news: Vec<String> = ["max", "high", "oom"]
            .iter()
            .map(|key| {
                let new = count(&after, key).saturating_sub(count(&before, key));
                format!("{key} {new}")
            })
            .collect();
        r.info(format!(
            "unpack {name}: memory.events new during the unpack: {}",
            news.join(", ")
        ));
    } else {
        r.info(format!("unpack {name}: memory not measured"));
    }
    Some(unpacked)
}

#[cfg(test)]
mod tests {
    use std::{os::unix::fs::PermissionsExt, sync::Mutex};

    use trss_archive::{child::Answer, Member, Refusal};

    use super::*;

    /// Held while a test writes a script and runs it: a child forked meanwhile
    /// by another test's thread would keep the script busy (ETXTBSY).
    static SPAWN: Mutex<()> = Mutex::new(());

    /// A shell script that prints `line` as its answer, whatever it is given.
    fn stand_in(dir: &Path, name: &str, line: &str) -> PathBuf {
        let path = dir.join(name);
        fs::write(&path, format!("#!/bin/sh\ncat <<'EOF'\n{line}\nEOF\n")).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
        path
    }

    fn answer(answer: &Answer) -> String {
        serde_json::to_string(answer).unwrap()
    }

    #[test]
    fn a_stand_in_program_gives_each_kind_of_outcome() {
        let _spawn = SPAWN.lock().unwrap_or_else(|e| e.into_inner());
        let dir = tempfile::tempdir().unwrap();
        let archive = dir.path().join("a.zip");
        fs::write(&archive, vec![0u8; 2048]).unwrap();
        let parts = [archive];
        let out = dir.path().join("out");
        let mut r = Report::default();

        let done = stand_in(
            dir.path(),
            "done.sh",
            &answer(&Answer::Done(vec![Member {
                path: "a.ass".to_owned(),
                file: "0".to_owned(),
                size: 5,
                sha256: "ab".repeat(32),
            }])),
        );
        let unpacked = measure(&mut r, None, &Unpacker::new(done), &parts, &out);
        assert!(matches!(unpacked, Some(Unpacked::Done(ref m)) if m.len() == 1));

        let refused = stand_in(
            dir.path(),
            "refused.sh",
            &answer(&Answer::Refused(Refusal::Ratio)),
        );
        let unpacked = measure(&mut r, None, &Unpacker::new(refused), &parts, &out);
        assert_eq!(unpacked, Some(Unpacked::Refused(Refusal::Ratio)));

        let garbage = stand_in(dir.path(), "garbage.sh", "not an answer");
        let unpacked = measure(&mut r, None, &Unpacker::new(garbage), &parts, &out);
        assert!(matches!(unpacked, Some(Unpacked::Failed(_))));

        // An outcome is not a check: bombs are expected to be refused.
        assert_eq!(r.failed(), 0);
    }

    #[test]
    fn an_input_that_is_missing_fails_and_is_not_unpacked() {
        let dir = tempfile::tempdir().unwrap();
        let program = stand_in(dir.path(), "never.sh", "");
        let mut r = Report::default();
        let unpacked = measure(
            &mut r,
            None,
            &Unpacker::new(program),
            &[dir.path().join("none.rar")],
            &dir.path().join("out"),
        );
        assert!(unpacked.is_none());
        assert_eq!(r.failed(), 1);
    }

    #[test]
    fn a_missing_program_is_one_failure_and_makes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let mut r = Report::default();
        let mut cleanup = Cleanup::default();
        run(
            &mut r,
            dir.path(),
            &[vec![dir.path().join("a.zip")]],
            &dir.path().join("no-such-trss-extract"),
            &mut cleanup,
        );
        assert_eq!(r.failed(), 1);
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 0);
    }

    #[test]
    fn the_test_folder_is_made_under_the_data_folder_and_removed() {
        let _spawn = SPAWN.lock().unwrap_or_else(|e| e.into_inner());
        let dir = tempfile::tempdir().unwrap();
        let archive = dir.path().join("a.zip");
        fs::write(&archive, b"x").unwrap();
        let data = dir.path().join("data");
        fs::create_dir(&data).unwrap();
        let program = stand_in(
            dir.path(),
            "refused.sh",
            &answer(&Answer::Refused(Refusal::Encrypted)),
        );
        let mut r = Report::default();
        let mut cleanup = Cleanup::default();
        run(&mut r, &data, &[vec![archive]], &program, &mut cleanup);
        assert_eq!(r.failed(), 0);
        let made: Vec<_> = fs::read_dir(&data).unwrap().collect();
        assert_eq!(made.len(), 1);
        let name = made[0].as_ref().unwrap().file_name();
        assert!(name.to_string_lossy().starts_with(files::PREFIX));
        let mut r = Report::default();
        cleanup.finish(&mut r);
        assert_eq!(fs::read_dir(&data).unwrap().count(), 0);
    }

    #[test]
    fn an_archive_is_named_by_its_first_part() {
        let parts = [
            PathBuf::from("/s/x.part1.rar"),
            PathBuf::from("/s/x.part2.rar"),
        ];
        assert_eq!(archive_name(&parts), "x.part1.rar");
    }
}
