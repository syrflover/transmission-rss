//! The state one extraction keeps across its archives, nested ones included,
//! and the files it writes (`docs/specs/subtitles.md`, 압축 해제의 격리와 한도).
//!
//! A format reads its archive's listing first and has every member admitted by
//! a [`Planner`] (path, kind, counts, declared sizes), so that an archive that
//! breaks a rule is refused before the first byte is written. A member that is
//! a file is then opened as a [`Sink`], which counts what is written against
//! the limits, hashes it, and writes it to `out/<index>`, never to a path the
//! archive names.

use std::{
    cell::RefCell,
    collections::HashSet,
    fs::{File, OpenOptions},
    io::{self, BufWriter, Read, Write},
    path::{Path, PathBuf},
    rc::Rc,
};

use sha2::{Digest, Sha256};

use crate::{
    name::check_path,
    source::{has_archive_extension, read_head, Format},
    ExtractError, Limits, Member, Refusal,
};

/// What a member is, as its format tells it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Kind {
    File,
    Dir,
    /// A symlink, a hard link or a reparse point.
    Link,
    /// A device, a FIFO, a socket or anything else that is not a regular file.
    Special,
}

/// A member admitted to be written: its full path, with the nested archives'
/// paths before it.
#[derive(Debug)]
pub(crate) struct Planned {
    pub path: String,
}

struct Inner {
    limits: Limits,
    out: PathBuf,
    /// `ratio * (the outer parts' sizes) + slack`: what may be written in all.
    ratio_cap: u64,
    members_seen: u64,
    written: u64,
    next_file: u64,
    /// The archives within archives being read, the outer one included.
    depth: u32,
    /// The path of the nested archive being read, with a `/`, or empty.
    prefix: String,
    members: Vec<Member>,
    files: HashSet<String>,
    dirs: HashSet<String>,
    /// The refusal or failure a write was stopped by, when a decoder's error
    /// hides it.
    stopped: Option<ExtractError>,
}

/// A handle on the state of one extraction. A decoder that writes through a
/// callback keeps one, so it is shared, and the extraction is single-threaded.
#[derive(Clone)]
pub(crate) struct Job(Rc<RefCell<Inner>>);

impl Job {
    pub(crate) fn new(limits: &Limits, out: &Path, outer_size: u64) -> Job {
        Job(Rc::new(RefCell::new(Inner {
            ratio_cap: limits
                .ratio
                .saturating_mul(outer_size)
                .saturating_add(limits.ratio_slack),
            limits: limits.clone(),
            out: out.to_owned(),
            members_seen: 0,
            written: 0,
            next_file: 0,
            depth: 1,
            prefix: String::new(),
            members: Vec::new(),
            files: HashSet::new(),
            dirs: HashSet::new(),
            stopped: None,
        })))
    }

    pub(crate) fn limits(&self) -> Limits {
        self.0.borrow().limits.clone()
    }

    /// Whether `more` members fit in the member limit, as a ZIP's end record or
    /// a 7z's header says before the members are listed.
    pub(crate) fn has_room_for(&self, more: u64) -> Result<(), Refusal> {
        let inner = self.0.borrow();
        if inner.members_seen.saturating_add(more) > inner.limits.members {
            return Err(Refusal::TooManyMembers {
                limit: inner.limits.members,
            });
        }
        Ok(())
    }

    /// The error for `what` failing on the archive being read: a refusal for
    /// the outer archive's parts, which the caller gave, and a failure of the
    /// machine for a nested archive, whose file this extraction wrote itself.
    pub(crate) fn input_error(&self, what: &str, error: io::Error) -> ExtractError {
        if self.0.borrow().depth > 1 {
            ExtractError::failed(&format!("푼 {what}"), error)
        } else {
            Refusal::corrupt(format!("{what}: {error}")).into()
        }
    }

    pub(crate) fn planner(&self) -> Planner {
        Planner {
            job: self.clone(),
            declared: 0,
        }
    }

    /// Notes `stopped` as what stopped the extraction, for a decoder to hand
    /// back as an error of its own, and returns an I/O error to stop it with.
    pub(crate) fn stop(&self, stopped: impl Into<ExtractError>) -> io::Error {
        self.0.borrow_mut().stopped.get_or_insert(stopped.into());
        io::Error::other("extraction stopped")
    }

    /// What a write stopped the extraction with, if one did, else `error`,
    /// which a decoder gave.
    pub(crate) fn resolve(&self, error: impl Into<ExtractError>) -> ExtractError {
        self.0
            .borrow_mut()
            .stopped
            .take()
            .unwrap_or_else(|| error.into())
    }

    /// The error for a failed [`Sink::copy`].
    pub(crate) fn copy_failed(&self, error: CopyError) -> ExtractError {
        match error {
            CopyError::Read(error) => {
                let dictionary = self.0.borrow().limits.dictionary;
                self.resolve(crate::failure::from_read(error, dictionary))
            }
            CopyError::Write(error) => error,
        }
    }

    /// Opens the file of an admitted member.
    pub(crate) fn open(&self, planned: Planned) -> Result<Sink, ExtractError> {
        let (index, file_path, sync_every) = {
            let mut inner = self.0.borrow_mut();
            let index = inner.next_file;
            inner.next_file += 1;
            (
                index,
                inner.out.join(index.to_string()),
                inner.limits.sync_every,
            )
        };
        let file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&file_path)
            .map_err(|error| {
                ExtractError::failed("압축을 풀 자리에 파일을 만들지 못했어요", error)
            })?;
        Ok(Sink {
            job: self.clone(),
            file: BufWriter::with_capacity(64 << 10, file),
            file_path,
            path: planned.path,
            index,
            size: 0,
            hasher: Sha256::new(),
            since_sync: 0,
            sync_every,
        })
    }

    /// Counts `n` more bytes of a member that has had `member` so far.
    fn account(&self, path: &str, member: u64, n: u64) -> Result<(), Refusal> {
        let mut inner = self.0.borrow_mut();
        if member.saturating_add(n) > inner.limits.member {
            return Err(Refusal::MemberTooLarge {
                path: path.to_owned(),
                limit: inner.limits.member,
            });
        }
        let written = inner.written.saturating_add(n);
        if written > inner.limits.total {
            return Err(Refusal::TooLarge {
                limit: inner.limits.total,
            });
        }
        if written > inner.ratio_cap {
            return Err(Refusal::Ratio);
        }
        inner.written = written;
        Ok(())
    }

    /// The members written, in the order they were.
    pub(crate) fn into_members(self) -> Vec<Member> {
        std::mem::take(&mut self.0.borrow_mut().members)
    }

    /// Unpacks the member just finished in place when it is an archive: its
    /// name ends in an archive extension and its bytes begin like one of the
    /// formats. Its members come out under its path, and it is removed and
    /// has no [`Member`] of its own.
    fn nest(&self, file_path: &Path, path: &str) -> Result<(), ExtractError> {
        let name = path.rsplit('/').next().unwrap_or(path);
        if !has_archive_extension(name) {
            return Ok(());
        }
        // The member's own file, which this extraction has just written.
        let head = read_head(file_path)
            .map_err(|error| ExtractError::failed("푼 파일을 읽지 못했어요", error))?;
        if Format::sniff(&head).is_none() {
            return Ok(());
        }
        let outer_prefix = {
            let mut inner = self.0.borrow_mut();
            if inner.depth + 1 > inner.limits.depth {
                return Err(Refusal::TooDeep {
                    limit: inner.limits.depth,
                }
                .into());
            }
            inner.members.pop();
            inner.depth += 1;
            std::mem::replace(&mut inner.prefix, format!("{path}/"))
        };
        let result = crate::unpack(self, &[file_path.to_owned()], name, false);
        {
            let mut inner = self.0.borrow_mut();
            inner.depth -= 1;
            inner.prefix = outer_prefix;
        }
        result?;
        std::fs::remove_file(file_path)
            .map_err(|error| ExtractError::failed("푼 파일을 지우지 못했어요", error))
    }
}

/// Admits the members of one archive.
pub(crate) struct Planner {
    job: Job,
    /// The sizes the members admitted so far declare.
    declared: u64,
}

impl Planner {
    /// Admits a member whose path inside the archive is `raw` (without a
    /// folder's trailing `/`). A file comes back as [`Planned`] to be opened; a
    /// folder is counted and not written; a link or a special file is refused.
    pub(crate) fn admit(
        &mut self,
        raw: &str,
        kind: Kind,
        declared: Option<u64>,
    ) -> Result<Option<Planned>, Refusal> {
        let mut inner = self.job.0.borrow_mut();
        let path = format!("{}{raw}", inner.prefix);
        check_path(&path, &inner.limits)?;
        match kind {
            Kind::Link => return Err(Refusal::Link { path }),
            Kind::Special => return Err(Refusal::Special { path }),
            Kind::File | Kind::Dir => {}
        }
        inner.members_seen += 1;
        if inner.members_seen > inner.limits.members {
            return Err(Refusal::TooManyMembers {
                limit: inner.limits.members,
            });
        }
        if kind == Kind::Dir {
            if inner.files.contains(&path) {
                return Err(Refusal::Duplicate { path });
            }
            inner.dirs.insert(path);
            return Ok(None);
        }
        if inner.files.contains(&path) || inner.dirs.contains(&path) {
            return Err(Refusal::Duplicate { path });
        }
        if let Some(size) = declared {
            if size > inner.limits.member {
                return Err(Refusal::MemberTooLarge {
                    path,
                    limit: inner.limits.member,
                });
            }
            self.declared = self.declared.saturating_add(size);
            let all = inner.written.saturating_add(self.declared);
            if all > inner.limits.total {
                return Err(Refusal::TooLarge {
                    limit: inner.limits.total,
                });
            }
            if all > inner.ratio_cap {
                return Err(Refusal::Ratio);
            }
        }
        inner.files.insert(path.clone());
        Ok(Some(Planned { path }))
    }
}

/// One member being written.
pub(crate) struct Sink {
    job: Job,
    file: BufWriter<File>,
    file_path: PathBuf,
    path: String,
    index: u64,
    size: u64,
    hasher: Sha256,
    since_sync: u64,
    sync_every: u64,
}

impl Sink {
    /// Writes `data`, counting it first: nothing is written past a limit.
    pub(crate) fn push(&mut self, data: &[u8]) -> Result<(), ExtractError> {
        self.job.account(&self.path, self.size, data.len() as u64)?;
        self.hasher.update(data);
        self.file.write_all(data).map_err(ExtractError::write)?;
        self.size += data.len() as u64;
        self.since_sync += data.len() as u64;
        if self.since_sync >= self.sync_every {
            self.since_sync = 0;
            self.sync_data()?;
        }
        Ok(())
    }

    /// Flushes the written bytes to the disk and drops them from the page
    /// cache, so a big member does not fill the container's memory with it.
    fn sync_data(&mut self) -> Result<(), ExtractError> {
        self.file.flush().map_err(ExtractError::write)?;
        let file = self.file.get_ref();
        rustix::fs::fdatasync(file).map_err(ExtractError::write)?;
        // A failed hint costs nothing but cache.
        let _ = rustix::fs::fadvise(file, 0, None, rustix::fs::Advice::DontNeed);
        Ok(())
    }

    /// Reads `reader` to its end into the member.
    pub(crate) fn copy(&mut self, reader: &mut dyn Read) -> Result<(), CopyError> {
        let mut buffer = vec![0u8; 64 << 10];
        loop {
            let n = match reader.read(&mut buffer) {
                Ok(0) => return Ok(()),
                Ok(n) => n,
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                Err(error) => return Err(CopyError::Read(error)),
            };
            self.push(&buffer[..n]).map_err(CopyError::Write)?;
        }
    }

    /// Ends the member: syncs its file, records it, and unpacks it in place
    /// when it is an archive.
    pub(crate) fn finish(mut self) -> Result<(), ExtractError> {
        self.file.flush().map_err(ExtractError::write)?;
        self.file
            .get_ref()
            .sync_all()
            .map_err(ExtractError::write)?;
        let sha256 = self
            .hasher
            .finalize()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        let path = self.path;
        self.job.0.borrow_mut().members.push(Member {
            path: path.clone(),
            file: self.index.to_string(),
            size: self.size,
            sha256,
        });
        drop(self.file);
        self.job.nest(&self.file_path, &path)
    }
}

impl Write for Sink {
    fn write(&mut self, data: &[u8]) -> io::Result<usize> {
        self.push(data).map_err(|error| self.job.stop(error))?;
        Ok(data.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        self.file.flush()
    }
}

/// Where [`Sink::copy`] failed.
pub(crate) enum CopyError {
    /// The decoder could not be read.
    Read(io::Error),
    /// The member could not be written: a limit, or the disk.
    Write(ExtractError),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_file_that_cannot_be_made_is_a_failure_not_a_refusal() {
        // `out` was never made, as when the folder is gone or the disk is full.
        let dir = tempfile::tempdir().unwrap();
        let job = Job::new(&Limits::default(), &dir.path().join("missing"), 1);
        let planned = Planned {
            path: "a.ass".to_owned(),
        };
        let Err(ExtractError::Failed(message)) = job.open(planned) else {
            panic!("opened");
        };
        assert!(
            message.starts_with("압축을 풀 자리에 파일을 만들지 못했어요: "),
            "{message}"
        );
    }

    #[test]
    fn a_write_that_fails_is_a_failure_not_a_refusal() {
        // `/dev/full` takes a write and says the disk is full.
        let Ok(full) = OpenOptions::new().write(true).open("/dev/full") else {
            return;
        };
        let dir = tempfile::tempdir().unwrap();
        let job = Job::new(&Limits::default(), dir.path(), 1);
        let mut sink = job
            .open(Planned {
                path: "a.ass".to_owned(),
            })
            .unwrap();
        // The member's own file stands where the full one is, unbuffered.
        sink.file = BufWriter::with_capacity(1, full);
        let Err(ExtractError::Failed(message)) = sink.push(&[1, 2, 3]) else {
            panic!("written");
        };
        assert!(
            message.starts_with("압축을 풀 자리에 쓰지 못했어요: "),
            "{message}"
        );
        assert!(message.contains("No space left on device"), "{message}");
    }
}

#[cfg(test)]
mod input_tests {
    use super::*;

    #[test]
    fn a_nested_archives_own_file_failing_is_a_failure_and_an_input_part_is_a_refusal() {
        let dir = tempfile::tempdir().unwrap();
        let job = Job::new(&Limits::default(), dir.path(), 1);
        let error = || io::Error::from(io::ErrorKind::PermissionDenied);
        assert!(matches!(
            job.input_error("파일을 열지 못했어요", error()),
            ExtractError::Refused(Refusal::Corrupt { .. })
        ));
        job.0.borrow_mut().depth = 2;
        let ExtractError::Failed(message) = job.input_error("파일을 열지 못했어요", error())
        else {
            panic!("a refusal");
        };
        assert!(
            message.starts_with("푼 파일을 열지 못했어요: "),
            "{message}"
        );
    }
}
