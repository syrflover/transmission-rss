//! The limits, the members written and the refusals of an extraction
//! (`docs/specs/subtitles.md`, 압축 해제의 격리와 한도).

use std::fmt;

use serde::{Deserialize, Serialize};

/// The limits of one extraction. [`Limits::default`] is the spec's table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Limits {
    /// Files and folders, the whole nesting together. A ZIP's total in its
    /// end-of-central-directory record is checked before the crate reads the
    /// central directory. A 7z's header must be no longer than 512 bytes for
    /// each of them and 64 KiB more, which is checked before the crate parses
    /// it and bounds what a crafted header can make the crate allocate (it does
    /// not prove how many files the header declares); the 7z's own file count
    /// is checked once the crate has parsed the header.
    pub members: u64,
    /// Bytes written, the whole nesting together.
    pub total: u64,
    /// One member's bytes: its declared size is checked first, then the bytes
    /// are counted while they are written.
    pub member: u64,
    /// A RAR whose listing has a bigger member is refused before extracting.
    pub rar_member: u64,
    /// The written bytes may be this many times the outer parts' sizes...
    pub ratio: u64,
    /// ...and this many bytes more.
    pub ratio_slack: u64,
    /// Archives within archives, the outer one included.
    pub depth: u32,
    /// Components of a path.
    pub path_parts: usize,
    /// Bytes of a whole (nested) path.
    pub path_bytes: usize,
    /// Bytes of one component.
    pub name_bytes: usize,
    /// The dictionary an xz, 7z (LZMA, LZMA2, and PPMd memory) or RAR may ask for.
    pub dictionary: u64,
    /// `fdatasync` and `fadvise(DONTNEED)` of the written file every this many bytes.
    pub sync_every: u64,
}

impl Default for Limits {
    fn default() -> Limits {
        Limits {
            members: 2000,
            total: 512 << 20,
            member: 200 << 20,
            rar_member: 64 << 20,
            ratio: 100,
            ratio_slack: 16 << 20,
            depth: 3,
            path_parts: 16,
            path_bytes: 1024,
            name_bytes: 255,
            dictionary: 64 << 20,
            sync_every: 8 << 20,
        }
    }
}

/// One member written to the output folder.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Member {
    /// The `/`-separated path inside the archive. A member of a nested archive
    /// is `<inner archive path>/<its path>`, e.g. `sub/inner.zip/a.ass`.
    pub path: String,
    /// Its file name within the output folder: the decimal index `0`, `1`, ... in
    /// the order the members are written (a 7z's empty files come after the
    /// ones with data).
    pub file: String,
    pub size: u64,
    /// Lowercase hex.
    pub sha256: String,
}

/// Why an archive is not unpacked.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Refusal {
    TooManyMembers { limit: u64 },
    TooLarge { limit: u64 },
    MemberTooLarge { path: String, limit: u64 },
    Ratio,
    TooDeep { limit: u32 },
    Path { path: String },
    Link { path: String },
    Special { path: String },
    Duplicate { path: String },
    Encrypted,
    Dictionary { limit: u64 },
    MissingVolume,
    Unsupported { what: String },
    Corrupt { detail: String },
}

/// Why an unpacking did not finish: the archive is refused ([`Refusal`]), or
/// the machine it runs on failed it (a full disk, no memory), which says
/// nothing about the archive.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExtractError {
    /// Something is wrong with the archive, or it is over a limit.
    Refused(Refusal),
    /// The unpacking could not go on for a reason of the local environment:
    /// the Korean reason, a few hundred characters at most.
    Failed(String),
}

impl From<Refusal> for ExtractError {
    fn from(refusal: Refusal) -> ExtractError {
        ExtractError::Refused(refusal)
    }
}

impl fmt::Display for ExtractError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ExtractError::Refused(refusal) => refusal.fmt(f),
            ExtractError::Failed(message) => f.write_str(message),
        }
    }
}

impl ExtractError {
    /// A [`ExtractError::Failed`] that says `what` went wrong because of
    /// `error`, which is escaped and cut to a length a message can carry.
    pub(crate) fn failed(what: &str, error: impl fmt::Display) -> ExtractError {
        ExtractError::Failed(format!("{what}: {}", shown(&error.to_string())))
    }

    /// The failure of writing to the folder the archive is unpacked into.
    pub(crate) fn write(error: impl fmt::Display) -> ExtractError {
        ExtractError::failed("압축을 풀 자리에 쓰지 못했어요", error)
    }

    /// The failure of the process running out of memory: its address space
    /// is limited ([`crate::child`]), and a decoder that asks for more fails
    /// or aborts the process.
    pub(crate) fn memory() -> ExtractError {
        ExtractError::Failed(MEMORY_LIMIT.to_owned())
    }
}

/// What is said of the child running into its address-space limit, whether it
/// says so itself or the parent sees it abort. The 256 MiB are
/// `crate::child`'s `ADDRESS_SPACE`.
pub(crate) const MEMORY_LIMIT: &str = "압축을 풀다가 메모리 한도(256 MB)에 닿았어요";

/// How long a path or a detail may be in a message.
const SHOWN: usize = 200;

/// A path for a message: control characters escaped, cut to [`SHOWN`] characters.
fn shown(text: &str) -> String {
    let mut out = String::new();
    for (i, c) in text.chars().enumerate() {
        if i == SHOWN {
            out.push('…');
            break;
        }
        if c.is_control() {
            out.extend(c.escape_default());
        } else {
            out.push(c);
        }
    }
    out
}

impl fmt::Display for Refusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Refusal::TooManyMembers { limit } => write!(f, "멤버가 {limit}개를 넘어요"),
            Refusal::TooLarge { limit } => write!(f, "풀린 크기가 {} MB를 넘어요", limit >> 20),
            Refusal::MemberTooLarge { path, limit } => {
                write!(
                    f,
                    "{}: 멤버 하나가 {} MB를 넘어요",
                    shown(path),
                    limit >> 20
                )
            }
            Refusal::Ratio => f.write_str("풀린 크기가 압축 파일 크기에 비해 너무 커요"),
            Refusal::TooDeep { limit } => {
                write!(f, "압축 파일 안의 압축 파일이 {limit}단을 넘어요")
            }
            Refusal::Path { path } => write!(f, "{}: 풀 수 없는 경로예요", shown(path)),
            Refusal::Link { path } => write!(f, "{}: 링크라서 풀지 않아요", shown(path)),
            Refusal::Special { path } => {
                write!(
                    f,
                    "{}: 장치나 FIFO 같은 특수 파일이라 풀지 않아요",
                    shown(path)
                )
            }
            Refusal::Duplicate { path } => {
                write!(f, "{}: 같은 경로의 멤버가 여럿이에요", shown(path))
            }
            Refusal::Encrypted => f.write_str("암호가 걸려 있어요"),
            Refusal::Dictionary { limit } => write!(f, "사전 크기가 {} MB를 넘어요", limit >> 20),
            Refusal::MissingVolume => f.write_str("나뉜 압축 파일의 조각이 모자라요"),
            Refusal::Unsupported { what } => {
                write!(f, "지원하지 않는 형식이에요: {}", shown(what))
            }
            Refusal::Corrupt { detail } => {
                write!(f, "압축 파일을 읽지 못했어요: {}", shown(detail))
            }
        }
    }
}

impl Refusal {
    /// A [`Refusal::Corrupt`] with `detail` cut to a length a message can carry.
    pub(crate) fn corrupt(detail: impl fmt::Display) -> Refusal {
        let detail = detail.to_string();
        Refusal::Corrupt {
            detail: detail.chars().take(SHOWN).collect(),
        }
    }

    pub(crate) fn unsupported(what: impl fmt::Display) -> Refusal {
        let what = what.to_string();
        Refusal::Unsupported {
            what: what.chars().take(SHOWN).collect(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn messages_escape_control_characters_and_cut_long_paths() {
        let refusal = Refusal::Path {
            path: "a\nb".to_owned(),
        };
        assert_eq!(refusal.to_string(), "a\\nb: 풀 수 없는 경로예요");
        let long = Refusal::Link {
            path: "가".repeat(500),
        };
        let text = long.to_string();
        assert!(text.starts_with(&"가".repeat(200)));
        assert!(text.contains('…'));
        assert!(!text.contains(&"가".repeat(201)));
    }

    #[test]
    fn the_memory_message_names_the_limit_of_the_child() {
        assert_eq!(crate::child::ADDRESS_SPACE, 256 << 20);
        assert!(MEMORY_LIMIT.contains("256 MB"));
    }

    #[test]
    fn limits_show_in_mebibytes() {
        assert_eq!(
            Refusal::MemberTooLarge {
                path: "x".to_owned(),
                limit: 64 << 20
            }
            .to_string(),
            "x: 멤버 하나가 64 MB를 넘어요"
        );
        assert_eq!(
            Refusal::Dictionary { limit: 64 << 20 }.to_string(),
            "사전 크기가 64 MB를 넘어요"
        );
    }
}
