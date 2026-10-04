//! The container's memory cgroup (cgroup v2) and a sampler of its use.

use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc,
    },
    thread::{self, JoinHandle},
    time::Duration,
};

/// Where a container sees its own cgroup.
const ROOT: &str = "/sys/fs/cgroup";

/// How often the sampler looks. Shorter than the writeback of a few MiB, so
/// that the peak is close to the real one.
const EVERY: Duration = Duration::from_millis(5);

#[derive(Clone)]
pub struct Memcg {
    dir: PathBuf,
}

/// `key value` lines (`memory.stat`, `memory.events`).
pub fn parse_keyed(text: &str) -> BTreeMap<String, u64> {
    text.lines()
        .filter_map(|line| {
            let (key, value) = line.split_once(' ')?;
            Some((key.to_owned(), value.trim().parse().ok()?))
        })
        .collect()
}

impl Memcg {
    /// The cgroup of this process when it is cgroup v2 with the memory
    /// controller; `None` otherwise.
    pub fn open() -> Option<Memcg> {
        Memcg::at(Path::new(ROOT))
    }

    fn at(dir: &Path) -> Option<Memcg> {
        dir.join("memory.current").is_file().then(|| Memcg {
            dir: dir.to_owned(),
        })
    }

    /// A one-value file as written (`max` for no limit), or why not.
    pub fn text(&self, name: &str) -> String {
        match fs::read_to_string(self.dir.join(name)) {
            Ok(text) => text.trim().to_owned(),
            Err(e) => format!("unreadable ({e})"),
        }
    }

    fn number(&self, name: &str) -> Option<u64> {
        fs::read_to_string(self.dir.join(name))
            .ok()?
            .trim()
            .parse()
            .ok()
    }

    pub fn keyed(&self, name: &str) -> BTreeMap<String, u64> {
        fs::read_to_string(self.dir.join(name))
            .map(|text| parse_keyed(&text))
            .unwrap_or_default()
    }

    /// Starts looking at the memory use until [`Sampler::stop`].
    pub fn sample(&self) -> Sampler {
        let stop = Arc::new(AtomicBool::new(false));
        let peaks = Arc::new(Peaks::default());
        let handle = thread::spawn({
            let (memcg, stop, peaks) = (self.clone(), stop.clone(), peaks.clone());
            move || {
                while !stop.load(Ordering::Relaxed) {
                    memcg.look(&peaks);
                    thread::sleep(EVERY);
                }
                memcg.look(&peaks);
            }
        });
        Sampler {
            stop,
            peaks,
            handle,
        }
    }

    fn look(&self, peaks: &Peaks) {
        if let Some(current) = self.number("memory.current") {
            peaks.current.fetch_max(current, Ordering::Relaxed);
        }
        let stat = self.keyed("memory.stat");
        for (key, peak) in [
            ("file", &peaks.file),
            ("anon", &peaks.anon),
            ("file_dirty", &peaks.dirty),
            ("file_writeback", &peaks.writeback),
        ] {
            if let Some(value) = stat.get(key) {
                peak.fetch_max(*value, Ordering::Relaxed);
            }
        }
    }
}

#[derive(Default)]
struct Peaks {
    current: AtomicU64,
    file: AtomicU64,
    anon: AtomicU64,
    dirty: AtomicU64,
    writeback: AtomicU64,
}

pub struct Sampler {
    stop: Arc<AtomicBool>,
    peaks: Arc<Peaks>,
    handle: JoinHandle<()>,
}

/// The largest values seen, in bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Peak {
    pub current: u64,
    pub file: u64,
    pub anon: u64,
    pub dirty: u64,
    pub writeback: u64,
}

impl Sampler {
    pub fn stop(self) -> Peak {
        self.stop.store(true, Ordering::Relaxed);
        let _ = self.handle.join();
        let load = |value: &AtomicU64| value.load(Ordering::Relaxed);
        Peak {
            current: load(&self.peaks.current),
            file: load(&self.peaks.file),
            anon: load(&self.peaks.anon),
            dirty: load(&self.peaks.dirty),
            writeback: load(&self.peaks.writeback),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keyed_lines_are_read_as_numbers() {
        let stat = parse_keyed("anon 4096\nfile 8192\nlow 0 \nbad line here\nnone x\n");
        assert_eq!(stat["anon"], 4096);
        assert_eq!(stat["file"], 8192);
        assert_eq!(stat["low"], 0);
        assert_eq!(stat.len(), 3);
    }

    #[test]
    fn the_sampler_keeps_the_largest_values_it_saw() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("memory.current"), "1000\n").unwrap();
        fs::write(
            dir.path().join("memory.stat"),
            "anon 10\nfile 700\nfile_dirty 5\n",
        )
        .unwrap();
        let memcg = Memcg::at(dir.path()).unwrap();

        let sampler = memcg.sample();
        thread::sleep(Duration::from_millis(30));
        fs::write(dir.path().join("memory.current"), "400\n").unwrap();
        thread::sleep(Duration::from_millis(30));
        let peak = sampler.stop();

        assert_eq!(peak.current, 1000);
        assert_eq!((peak.anon, peak.file, peak.dirty), (10, 700, 5));
        assert_eq!(peak.writeback, 0);
    }

    #[test]
    fn a_folder_without_memory_files_is_no_memcg() {
        let dir = tempfile::tempdir().unwrap();
        assert!(Memcg::at(dir.path()).is_none());
    }
}
