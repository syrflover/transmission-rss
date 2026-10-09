//! Why an episode is not placed in the season, in the words the screens show
//! (`docs/specs/subtitles.md`, 파일의 회차): the sentences built from an
//! episode text, the season episode a [`Mapping`] puts it on, and the season's
//! episode count.
//!
//! The same condition is worded a little differently where it is shown: a
//! candidate's job says `후보의`, a file of a received package does not, and
//! the stored subtitles a changed mapping moves say `정했어요` where the other
//! two say `정해 두었어요`. [`Wording`] names the three places, and the
//! differences live in one `match` each, so a wording is changed here and
//! nowhere else. The screens and the web tests read these sentences
//! literally; `reason/tests.rs` has each of them as it is shown.

use super::{Mapped, Mapping};

/// Where a sentence is shown, which words it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Wording {
    /// The episode of a subtitle candidate of Anissia, for its job.
    Candidate,
    /// A file of a received package or a person's upload.
    File,
    /// A stored subtitle or an unfinished row, when the mapping changes.
    Stored,
}

/// `1–12화`, or `1화부터` when the season's episode count is unknown.
pub fn season_range(total: Option<u32>) -> String {
    match total {
        Some(n) => format!("1–{n}화"),
        None => "1화부터".to_owned(),
    }
}

/// Whether `episode` is one of the season's, which has `total` episodes when
/// that is known.
pub fn in_season(episode: i64, total: Option<u32>) -> bool {
    episode >= 1 && total.is_none_or(|n| episode <= i64::from(n))
}

/// Why nothing is placed while the source's mapping is undecided.
pub const UNDECIDED: &str = "이 제작자의 회차 대응이 아직 미정이에요";

/// Where `mapping` puts the episode text `text`, or why it puts it nowhere. The
/// episode may lie outside the season: [`in_season`] and [`outside_season`]
/// say that.
pub fn place(mapping: &Mapping, text: &str, wording: Wording) -> Result<i64, String> {
    let label = trss_core::episode::episode_label(text);
    match mapping.season_episode(text) {
        Mapped::Episode(n) => Ok(n),
        Mapped::NotReceived => Err(match wording {
            Wording::Candidate => {
                format!("회차 대응이 후보의 {label}를 받지 않는 회차로 정해 두었어요")
            }
            Wording::File => format!("회차 대응이 {label}를 받지 않는 회차로 정해 두었어요"),
            Wording::Stored => format!("회차 대응이 {label}를 받지 않는 회차로 정했어요"),
        }),
        Mapped::Unmapped if mapping.decided_offset().is_none() => Err(UNDECIDED.to_owned()),
        Mapped::Unmapped => Err(match wording {
            Wording::Candidate => {
                format!("후보의 회차 {label}는 회차 대응으로 옮길 수 없는 회차예요")
            }
            Wording::File | Wording::Stored => {
                format!("{label}는 회차 대응으로 옮길 수 없는 회차예요")
            }
        }),
    }
}

/// An episode text that is no whole number (`5.5`, `SP`) cannot be put on a
/// season episode without a mapping to say so.
pub fn not_whole(text: &str, wording: Wording) -> String {
    let label = trss_core::episode::episode_label(text);
    match wording {
        Wording::Candidate => {
            format!("후보의 회차 {label}는 정수 회차가 아니라 시즌의 회차로 옮기지 못했어요")
        }
        Wording::File | Wording::Stored => {
            format!("{label}는 정수 회차가 아니라 시즌의 회차로 정하지 못했어요")
        }
    }
}

/// The text `text` got the season episode `episode` through the mapping (or as
/// it is), and that is not one of the season's.
pub fn outside_season(text: &str, episode: i64, total: Option<u32>, wording: Wording) -> String {
    let label = trss_core::episode::episode_label(text);
    let range = season_range(total);
    match wording {
        Wording::Candidate => {
            format!("후보의 {label}를 옮긴 시즌 {episode}화가 시즌의 {range} 밖이에요")
        }
        Wording::File => format!("{label}가 시즌의 {range} 밖이에요"),
        Wording::Stored => format!("{label}를 옮긴 시즌 {episode}화가 시즌의 {range} 밖이에요"),
    }
}

/// [`outside_season`] for a file of a package placed with others, whose
/// episode outside the season means it is another season's file.
pub fn another_seasons_file(text: &str, total: Option<u32>) -> String {
    format!(
        "{}가 시즌의 {} 밖이라 다른 시즌의 파일로 보여요",
        trss_core::episode::episode_label(text),
        season_range(total)
    )
}

/// The names of a package number its files in Anissia's way or the season's
/// and the mapping would put `text` somewhere else than its own number.
pub fn numbering_unclear(text: &str) -> String {
    format!(
        "파일 이름의 {}가 Anissia의 회차인지 시즌의 회차인지 정하지 못했어요",
        trss_core::episode::episode_label(text)
    )
}

/// Why a person is refused the episode `episode` they placed a row on at 배치
/// 확인, which is outside the season.
pub fn placed_outside_season(episode: i64, total: Option<u32>) -> String {
    match total {
        Some(n) => format!("{episode}화는 이 시즌의 1–{n}화 밖이에요."),
        None => format!("{episode}화는 회차가 될 수 없어요."),
    }
}

#[cfg(test)]
mod tests;
