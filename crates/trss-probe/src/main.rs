//! Checks, on a real disk and under the worker's limits, the file behavior
//! that trss's archiving and subtitle placing rely on, and prints a plain-text
//! report (ticket 0060).
//!
//! It makes its test folders under the folders it is given, named
//! `.trss-probe-<pid>-<n>`, and removes them (and nothing else) when it ends.
//! It never reads or changes any other file. Run it as the user and under the
//! memory limit of the worker's container; `deploy/probe.sh` does that.

mod cgroup;
mod files;
mod memtest;
mod mountinfo;
mod report;
mod unpack;

use std::{fs, path::PathBuf, process::ExitCode};

use files::{Cleanup, Owner};
use report::Report;
use rustix::process::{getgid, getuid};

const USAGE: &str = "\
usage: trss-probe [options]

  --media DIR         a folder on the media disk to make the test folder in
                      (default /downloads)
  --data DIR          a folder on the app data disk, for the data-to-media
                      rename (default /data)
  --work DIR          an existing work folder that Transmission made, to make a
                      test subfolder in (repeatable; the probe adds nothing
                      else there and removes the subfolder)
  --size-mib N        the size of the memory test's file (default 300; 0 skips
                      the test)
  --unpack FILE[,FILE...]
                      an archive to unpack as the worker does, under the memory
                      limit, in a test folder under --data; for a split one,
                      its volumes in order, comma-separated (repeatable)
  --extract PATH      the trss-extract program (default: trss-extract beside
                      this program)
  --owner UID:GID     the owner the files are expected to have (default 1000:1000)
  -h, --help          this text

Exit status: 0 when every check passed, 1 when one failed, 2 for a bad option.";

struct Options {
    media: PathBuf,
    data: PathBuf,
    work: Vec<PathBuf>,
    size_mib: u64,
    owner: Owner,
    /// The volumes of each archive to unpack.
    unpack: Vec<Vec<PathBuf>>,
    extract: Option<PathBuf>,
}

/// `Ok(None)` is a request for the usage text.
fn parse_args(args: impl IntoIterator<Item = String>) -> Result<Option<Options>, String> {
    let mut options = Options {
        media: PathBuf::from("/downloads"),
        data: PathBuf::from("/data"),
        work: Vec::new(),
        size_mib: 300,
        owner: Owner {
            uid: 1000,
            gid: 1000,
        },
        unpack: Vec::new(),
        extract: None,
    };
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        let mut value = |name: &str| args.next().ok_or_else(|| format!("{name} needs a value"));
        match arg.as_str() {
            "-h" | "--help" => return Ok(None),
            "--media" => options.media = value("--media")?.into(),
            "--data" => options.data = value("--data")?.into(),
            "--work" => options.work.push(value("--work")?.into()),
            "--size-mib" => {
                options.size_mib = value("--size-mib")?
                    .parse()
                    .map_err(|_| "--size-mib needs a whole number".to_owned())?;
            }
            "--unpack" => {
                let text = value("--unpack")?;
                let parts: Vec<PathBuf> = text.split(',').map(PathBuf::from).collect();
                if parts.iter().any(|part| part.as_os_str().is_empty()) {
                    return Err("--unpack needs FILE[,FILE...], with no empty file name".to_owned());
                }
                options.unpack.push(parts);
            }
            "--extract" => options.extract = Some(value("--extract")?.into()),
            "--owner" => {
                let text = value("--owner")?;
                let parsed = text
                    .split_once(':')
                    .and_then(|(u, g)| Some((u.parse().ok()?, g.parse().ok()?)));
                let Some((uid, gid)) = parsed else {
                    return Err("--owner needs UID:GID, for example 1000:1000".to_owned());
                };
                options.owner = Owner { uid, gid };
            }
            other => return Err(format!("unknown option {other}")),
        }
    }
    Ok(Some(options))
}

/// `2026-10-04T14:07:00Z` from the clock.
fn utc_now() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    iso_utc(secs)
}

fn iso_utc(secs: u64) -> String {
    let (days, rest) = (secs / 86_400, secs % 86_400);
    // Days since 1970-01-01 to a civil date (Howard Hinnant's algorithm).
    let z = days as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        rest / 3_600,
        rest % 3_600 / 60,
        rest % 60
    )
}

fn main() -> ExitCode {
    let options = match parse_args(std::env::args().skip(1)) {
        Ok(Some(options)) => options,
        Ok(None) => {
            println!("{USAGE}");
            return ExitCode::SUCCESS;
        }
        Err(message) => {
            eprintln!("trss-probe: {message}\n\n{USAGE}");
            return ExitCode::from(2);
        }
    };
    let mut r = Report::default();
    run(&mut r, &options);
    r.summary();
    if r.failed() == 0 {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(1)
    }
}

/// `trss-extract` beside this program, where `deploy/probe.sh` mounts it.
fn default_extract() -> PathBuf {
    std::env::current_exe()
        .unwrap_or_else(|_| PathBuf::from("trss-probe"))
        .with_file_name("trss-extract")
}

fn run(r: &mut Report, options: &Options) {
    r.section("trss-probe");
    r.info(format!("time {}", utc_now()));
    r.info(format!("version {}", env!("CARGO_PKG_VERSION")));

    r.section("System");
    for (name, path) in [
        ("kernel", "/proc/sys/kernel/osrelease"),
        ("kernel build", "/proc/version"),
    ] {
        r.info(format!(
            "{name}: {}",
            fs::read_to_string(path)
                .map_or_else(|e| format!("unreadable ({e})"), |t| t.trim().to_owned())
        ));
    }
    r.info(format!(
        "running as uid {} gid {}",
        getuid().as_raw(),
        getgid().as_raw()
    ));
    r.info(format!(
        "expected owner of files made here: {}:{}",
        options.owner.uid, options.owner.gid
    ));

    r.section("Filesystems");
    let mounts = fs::read_to_string("/proc/self/mountinfo")
        .map(|text| mountinfo::parse(&text))
        .unwrap_or_default();
    files::describe(r, "media", &options.media, &mounts);
    files::describe(r, "data", &options.data, &mounts);
    for work in &options.work {
        files::describe(r, "work", work, &mounts);
    }

    let mut cleanup = Cleanup::default();
    let name = files::unique_name();

    r.section("Media folder");
    let media_dir = options.media.join(&name);
    cleanup.track(&media_dir);
    let made = fs::create_dir(&media_dir);
    r.check(
        made.is_ok(),
        "media: make the test folder",
        match &made {
            Ok(()) => media_dir.display().to_string(),
            Err(e) => format!("{}: {e}", media_dir.display()),
        },
    );
    if made.is_ok() {
        files::same_filesystem(r, "media", &media_dir, options.owner);
    }

    r.section("Data folder");
    let data_dir = options.data.join(&name);
    cleanup.track(&data_dir);
    let made = fs::create_dir(&data_dir);
    r.check(
        made.is_ok(),
        "data: make the test folder",
        match &made {
            Ok(()) => data_dir.display().to_string(),
            Err(e) => format!("{}: {e}", data_dir.display()),
        },
    );
    if made.is_ok() {
        files::same_filesystem(r, "data", &data_dir, options.owner);
    }

    if media_dir.is_dir() && data_dir.is_dir() {
        r.section("Between the data folder and the media");
        let different = match (
            files::mount_id(&mounts, &data_dir),
            files::mount_id(&mounts, &media_dir),
        ) {
            (Some(a), Some(b)) => Some(a != b),
            _ => None,
        };
        files::across_mounts(r, &data_dir, &media_dir, different);
    }

    for work in &options.work {
        r.section("Work folder");
        files::work_folder(r, work, options.owner, &mut cleanup);
    }

    if options.size_mib > 0 && media_dir.is_dir() {
        r.section("Memory limit");
        memtest::run(r, &media_dir, options.size_mib);
    }

    if !options.unpack.is_empty() {
        r.section("Unpacking");
        let program = options.extract.clone().unwrap_or_else(default_extract);
        unpack::run(r, &options.data, &options.unpack, &program, &mut cleanup);
    }

    r.section("Cleanup");
    cleanup.finish(r);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn the_defaults_are_the_workers_mounts_and_owner() {
        let options = parse_args(args(&[])).unwrap().unwrap();
        assert_eq!(options.media, PathBuf::from("/downloads"));
        assert_eq!(options.data, PathBuf::from("/data"));
        assert!(options.work.is_empty());
        assert_eq!(options.size_mib, 300);
        assert!(options.unpack.is_empty());
        assert_eq!(options.extract, None);
        assert_eq!(
            options.owner,
            Owner {
                uid: 1000,
                gid: 1000
            }
        );
    }

    #[test]
    fn options_are_read() {
        let options = parse_args(args(&[
            "--media",
            "/m",
            "--data",
            "/d",
            "--work",
            "/w1",
            "--work",
            "/w2",
            "--size-mib",
            "64",
            "--owner",
            "5:6",
        ]))
        .unwrap()
        .unwrap();
        assert_eq!(options.media, PathBuf::from("/m"));
        assert_eq!(options.data, PathBuf::from("/d"));
        assert_eq!(options.work, [PathBuf::from("/w1"), PathBuf::from("/w2")]);
        assert_eq!(options.size_mib, 64);
        assert_eq!(options.owner, Owner { uid: 5, gid: 6 });
    }

    #[test]
    fn archives_to_unpack_are_read_with_their_volumes() {
        let options = parse_args(args(&[
            "--unpack",
            "/s/a.zip",
            "--unpack",
            "/s/b.part1.rar,/s/b.part2.rar",
            "--extract",
            "/x/trss-extract",
        ]))
        .unwrap()
        .unwrap();
        assert_eq!(
            options.unpack,
            [
                vec![PathBuf::from("/s/a.zip")],
                vec![
                    PathBuf::from("/s/b.part1.rar"),
                    PathBuf::from("/s/b.part2.rar")
                ]
            ]
        );
        assert_eq!(options.extract, Some(PathBuf::from("/x/trss-extract")));
    }

    #[test]
    fn the_extract_program_defaults_to_the_one_beside_the_probe() {
        let program = default_extract();
        assert_eq!(program.file_name().unwrap(), "trss-extract");
        assert_eq!(program.parent(), std::env::current_exe().unwrap().parent());
    }

    #[test]
    fn a_bad_option_is_refused_and_help_asks_for_the_usage() {
        assert!(parse_args(args(&["--nope"])).is_err());
        assert!(parse_args(args(&["--media"])).is_err());
        assert!(parse_args(args(&["--owner", "1000"])).is_err());
        assert!(parse_args(args(&["--size-mib", "many"])).is_err());
        assert!(parse_args(args(&["--unpack"])).is_err());
        assert!(parse_args(args(&["--unpack", ""])).is_err());
        assert!(parse_args(args(&["--unpack", "a.rar,,b.rar"])).is_err());
        assert!(parse_args(args(&["--unpack", ",a.rar"])).is_err());
        assert!(parse_args(args(&["--extract"])).is_err());
        assert!(parse_args(args(&["--help"])).unwrap().is_none());
    }

    #[test]
    fn the_time_is_written_in_utc() {
        assert_eq!(iso_utc(0), "1970-01-01T00:00:00Z");
        assert_eq!(iso_utc(1_791_123_100), "2026-10-04T14:11:40Z");
        // The day after a leap day.
        assert_eq!(iso_utc(1_709_251_200 + 86_399), "2024-03-01T23:59:59Z");
    }
}
