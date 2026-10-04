//! A big file written and read back under the container's memory limit, with
//! the cgroup's own numbers: the page cache of a copy is what the limit
//! meets (ticket 0058, the kernel's retry of a charge that cannot be made).

use std::{
    fs::{self, File},
    io::{Read, Write},
    path::Path,
    time::Instant,
};

use rustix::fs::{fadvise, Advice};

use crate::{
    cgroup::{Memcg, Peak},
    report::Report,
};

const MIB: u64 = 1024 * 1024;

/// A chunk's bytes: known from its number alone, so that the read can check
/// them without keeping the file.
fn fill(buf: &mut [u8], chunk: u64) {
    for (i, byte) in buf.iter_mut().enumerate() {
        *byte = (i as u64).wrapping_mul(31).wrapping_add(chunk) as u8;
    }
}

fn mib(bytes: u64) -> String {
    format!("{:.1} MiB", bytes as f64 / MIB as f64)
}

fn peak_text(peak: &Peak) -> String {
    format!(
        "peak memory.current {}, memory.stat file {}, anon {}, file_dirty {}, file_writeback {}",
        mib(peak.current),
        mib(peak.file),
        mib(peak.anon),
        mib(peak.dirty),
        mib(peak.writeback)
    )
}

fn rate(bytes: u64, started: Instant) -> String {
    let secs = started.elapsed().as_secs_f64().max(0.001);
    format!("{secs:.1} s, {:.0} MiB/s", bytes as f64 / MIB as f64 / secs)
}

/// Writes `size_mib` MiB into `dir`, fsyncs it, drops its cache, reads it
/// back and compares it, sampling the memory cgroup throughout.
pub fn run(r: &mut Report, dir: &Path, size_mib: u64) {
    let Some(memcg) = Memcg::open() else {
        r.info("no cgroup v2 memory files under /sys/fs/cgroup: the memory test is skipped");
        return;
    };
    r.info(format!(
        "memory.max {}, memory.high {}, memory.swap.max {}, memory.current {}",
        memcg.text("memory.max"),
        memcg.text("memory.high"),
        memcg.text("memory.swap.max"),
        memcg.text("memory.current")
    ));
    let events_before = memcg.keyed("memory.events");
    r.info(format!("memory.events before: {events_before:?}"));

    let path = dir.join("big.bin");
    let total = size_mib * MIB;
    let mut buf = vec![0u8; MIB as usize];

    r.info(format!(
        "writing {size_mib} MiB to {} (if the output stops here, the probe was most likely killed by the memory limit: look at `docker inspect` OOMKilled and the kernel log)",
        path.display()
    ));
    let sampler = memcg.sample();
    let started = Instant::now();
    let written = (|| -> std::io::Result<()> {
        let mut file = File::create(&path)?;
        for chunk in 0..size_mib {
            fill(&mut buf, chunk);
            file.write_all(&buf)?;
        }
        file.sync_all()
    })();
    let peak = sampler.stop();
    r.check(
        written.is_ok(),
        "memory: write and fsync",
        match &written {
            Ok(()) => format!("{}; {}", rate(total, started), peak_text(&peak)),
            Err(e) => e.to_string(),
        },
    );
    if written.is_err() {
        return;
    }

    // The written pages are clean now; dropping them makes the read come from
    // the disk, as the read of a file that was not just written does.
    let dropped = File::open(&path).and_then(|f| Ok(fadvise(&f, 0, None, Advice::DontNeed)?));
    r.info(match dropped {
        Ok(()) => "dropped the file's cached pages (posix_fadvise DONTNEED)".to_owned(),
        Err(e) => format!("could not drop the file's cached pages: {e}"),
    });

    let sampler = memcg.sample();
    let started = Instant::now();
    let read = (|| -> std::io::Result<Option<u64>> {
        let mut file = File::open(&path)?;
        let mut expected = vec![0u8; MIB as usize];
        for chunk in 0..size_mib {
            file.read_exact(&mut buf)?;
            fill(&mut expected, chunk);
            if buf != expected {
                return Ok(Some(chunk));
            }
        }
        Ok(None)
    })();
    let peak = sampler.stop();
    r.check(
        matches!(read, Ok(None)),
        "memory: read back and compare",
        match &read {
            Ok(None) => format!("{}; {}", rate(total, started), peak_text(&peak)),
            Ok(Some(chunk)) => format!("the bytes of MiB {chunk} differ from what was written"),
            Err(e) => e.to_string(),
        },
    );

    let _ = fs::remove_file(&path);

    let events_after = memcg.keyed("memory.events");
    r.info(format!("memory.events after: {events_after:?}"));
    let count = |events: &std::collections::BTreeMap<String, u64>, key: &str| {
        events.get(key).copied().unwrap_or(0)
    };
    let kills = count(&events_after, "oom_kill").saturating_sub(count(&events_before, "oom_kill"));
    r.check(
        kills == 0,
        "memory: oom_kill during the test",
        format!(
            "{} -> {} ({kills} new)",
            count(&events_before, "oom_kill"),
            count(&events_after, "oom_kill")
        ),
    );
    for key in ["max", "high", "oom"] {
        let new = count(&events_after, key).saturating_sub(count(&events_before, key));
        r.info(format!("memory.events {key}: {new} new during the test"));
    }
    let stat = memcg.keyed("memory.stat");
    r.info(format!(
        "after: memory.current {}, memory.stat file {} MiB, anon {} MiB",
        memcg.text("memory.current"),
        stat.get("file").copied().unwrap_or(0) / MIB,
        stat.get("anon").copied().unwrap_or(0) / MIB
    ));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_chunks_bytes_follow_from_its_number() {
        let (mut a, mut b) = (vec![0u8; 64], vec![0u8; 64]);
        fill(&mut a, 3);
        fill(&mut b, 3);
        assert_eq!(a, b);
        fill(&mut b, 4);
        assert_ne!(a, b);
    }
}
