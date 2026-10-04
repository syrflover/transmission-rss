//! The mount a folder is on, from `/proc/self/mountinfo`.

use std::path::{Path, PathBuf};

#[derive(Debug, PartialEq, Eq)]
pub struct Mount {
    pub id: u32,
    /// `major:minor` of the filesystem's device.
    pub device: String,
    /// The path inside the filesystem that is mounted here (a bind mount of a
    /// folder shows that folder's path on the host).
    pub root: String,
    pub point: PathBuf,
    pub options: String,
    pub fstype: String,
    pub source: String,
    pub super_options: String,
}

/// Reads one line of `mountinfo(5)`:
/// `36 35 98:0 /mnt1 /mnt2 rw,noatime master:1 - ext3 /dev/root rw`.
pub fn parse_line(line: &str) -> Option<Mount> {
    let mut fields = line.split(' ');
    let id = fields.next()?.parse().ok()?;
    let _parent = fields.next()?;
    let device = fields.next()?.to_owned();
    let root = unescape(fields.next()?);
    let point = unescape(fields.next()?);
    let options = fields.next()?.to_owned();
    // Optional fields (`master:1`, `shared:2`) end at a lone `-`.
    fields.by_ref().find(|f| *f == "-")?;
    let fstype = fields.next()?.to_owned();
    let source = unescape(fields.next()?);
    let super_options = fields.next().unwrap_or("").to_owned();
    Some(Mount {
        id,
        device,
        root,
        point: PathBuf::from(point),
        options,
        fstype,
        source,
        super_options,
    })
}

/// The kernel writes a space, tab, newline and backslash in a path as `\040`,
/// `\011`, `\012` and `\134`.
fn unescape(field: &str) -> String {
    let bytes = field.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'\\'
            && i + 3 < bytes.len()
            && bytes[i + 1..i + 4].iter().all(u8::is_ascii_digit)
        {
            let value = (bytes[i + 1] - b'0') as u32 * 64
                + (bytes[i + 2] - b'0') as u32 * 8
                + (bytes[i + 3] - b'0') as u32;
            out.push(value as u8);
            i += 4;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

pub fn parse(text: &str) -> Vec<Mount> {
    text.lines().filter_map(parse_line).collect()
}

/// The mount that holds `path` (an absolute, link-free path): the one with
/// the longest mount point that is a parent of it. Of several at one point
/// (mounted over each other) the last listed is the one in effect.
pub fn holding<'a>(mounts: &'a [Mount], path: &Path) -> Option<&'a Mount> {
    let mut best: Option<&Mount> = None;
    for mount in mounts {
        if !path.starts_with(&mount.point) {
            continue;
        }
        let longer =
            best.is_none_or(|b| mount.point.as_os_str().len() >= b.point.as_os_str().len());
        if longer {
            best = Some(mount);
        }
    }
    best
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "\
24 1 0:21 / / rw,relatime - overlay overlay rw,lowerdir=/a,upperdir=/b
31 24 8:17 /srv/media /downloads rw,noatime master:1 - xfs /dev/sdb1 rw,seclabel,attr2,inode64,noquota
32 24 8:2 /srv/trss\\040data /data rw,relatime - xfs /dev/sda2 rw,seclabel
33 31 8:17 /srv/media/sub /downloads/sub rw,noatime - xfs /dev/sdb1 rw
";

    #[test]
    fn a_line_gives_its_filesystem_and_options() {
        let mounts = parse(SAMPLE);
        assert_eq!(mounts.len(), 4);
        let media = &mounts[1];
        assert_eq!(media.id, 31);
        assert_eq!(media.device, "8:17");
        assert_eq!(media.root, "/srv/media");
        assert_eq!(media.point, Path::new("/downloads"));
        assert_eq!(media.options, "rw,noatime");
        assert_eq!(media.fstype, "xfs");
        assert_eq!(media.source, "/dev/sdb1");
        assert_eq!(media.super_options, "rw,seclabel,attr2,inode64,noquota");
    }

    #[test]
    fn an_escaped_space_in_a_path_is_read_back() {
        assert_eq!(parse(SAMPLE)[2].root, "/srv/trss data");
    }

    #[test]
    fn the_longest_mount_point_that_holds_a_path_wins() {
        let mounts = parse(SAMPLE);
        assert_eq!(
            holding(&mounts, Path::new("/downloads/a/b")).unwrap().id,
            31
        );
        assert_eq!(
            holding(&mounts, Path::new("/downloads/sub/x")).unwrap().id,
            33
        );
        assert_eq!(holding(&mounts, Path::new("/data/.x")).unwrap().id, 32);
        assert_eq!(holding(&mounts, Path::new("/elsewhere")).unwrap().id, 24);
        // A sibling that shares only a name prefix is not inside the mount.
        assert_eq!(holding(&mounts, Path::new("/downloads2/x")).unwrap().id, 24);
    }

    #[test]
    fn a_line_that_is_not_mountinfo_is_skipped() {
        assert!(parse_line("not a mount").is_none());
        assert!(parse_line("1 2 3:4 / /x rw").is_none());
    }
}
