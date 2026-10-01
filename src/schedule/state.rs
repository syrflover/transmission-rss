//! The status lines of a card on the weekly schedule
//! (`docs/specs/web-app.md`, 이번 주 편성): one for the video and, unless the
//! subscription takes no subtitles, one for the subtitle.

use serde::Serialize;

use crate::store::{channels::SubtitleMode, history::Millis};

/// The video line of a card.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum VideoState {
    /// `영상 받음`: the library has the episode's video.
    Received,
    /// `영상 받는 중`: Transmission is downloading the episode's torrent. The
    /// only video state that is emphasised.
    Downloading,
    /// `영상 대기`: it has aired and the video has not come.
    Waiting,
    /// `방영 전`: it has not aired yet.
    Upcoming,
    /// `받기 멈춤`: `영상 받기` is off, so nothing is received.
    Paused,
}

/// The subtitle line of a card.
///
/// `Downloading`, `AuthRequired` and `EpisodeUnconfirmed` are the states the
/// subtitle side will fill (`자막 받는 중`, `인증 필요`, `회차 확인 필요`);
/// nothing produces them yet, so the weekly schedule never sends them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SubtitleState {
    /// `자막 받음`: the library has a subtitle for the episode.
    Received,
    /// `자막 대기`: waiting for the creator; quiet.
    Waiting,
    /// `자막 받는 중`.
    Downloading,
    /// `인증 필요`: the user must solve a CAPTCHA; emphasised.
    AuthRequired,
    /// `회차 확인 필요`: a subtitle file whose episode is not decided;
    /// emphasised.
    EpisodeUnconfirmed,
}

/// What a card's status lines are made of.
#[derive(Debug, Clone, Copy)]
pub struct Facts {
    pub now: Millis,
    /// When the episode airs.
    pub instant: Millis,
    /// The rule is paused (`영상 받기` off).
    pub paused: bool,
    pub subtitles: SubtitleMode,
    /// The library has the episode's video / a subtitle for it.
    pub video_held: bool,
    pub subtitle_held: bool,
    /// Transmission is downloading a torrent of the episode.
    pub downloading: bool,
}

/// The video line. A paused subscription says so instead of anything else:
/// it does not receive, whatever the library has.
pub fn video(facts: &Facts) -> VideoState {
    if facts.paused {
        VideoState::Paused
    } else if facts.video_held {
        VideoState::Received
    } else if facts.downloading {
        VideoState::Downloading
    } else if facts.now < facts.instant {
        VideoState::Upcoming
    } else {
        VideoState::Waiting
    }
}

/// The subtitle line, or `None` when the card has none: the subscription takes
/// no subtitles (`받지 않음`), or its video is off, which leaves subtitles
/// nothing to wait for (`자막 받기` is disabled then).
pub fn subtitle(facts: &Facts) -> Option<SubtitleState> {
    if facts.paused || facts.subtitles == SubtitleMode::None {
        None
    } else if facts.subtitle_held {
        Some(SubtitleState::Received)
    } else {
        Some(SubtitleState::Waiting)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn facts() -> Facts {
        Facts {
            now: 1_000,
            instant: 500,
            paused: false,
            subtitles: SubtitleMode::Follow,
            video_held: false,
            subtitle_held: false,
            downloading: false,
        }
    }

    #[test]
    fn the_video_line_follows_the_library_then_the_download_then_the_clock() {
        assert_eq!(video(&facts()), VideoState::Waiting);
        assert_eq!(
            video(&Facts {
                instant: 2_000,
                ..facts()
            }),
            VideoState::Upcoming
        );
        // It is on air at this very moment: not before it.
        assert_eq!(
            video(&Facts {
                instant: 1_000,
                ..facts()
            }),
            VideoState::Waiting
        );
        assert_eq!(
            video(&Facts {
                downloading: true,
                ..facts()
            }),
            VideoState::Downloading
        );
        assert_eq!(
            video(&Facts {
                video_held: true,
                downloading: true,
                ..facts()
            }),
            VideoState::Received
        );
        // A video the library has before the broadcast time still counts.
        assert_eq!(
            video(&Facts {
                instant: 2_000,
                video_held: true,
                ..facts()
            }),
            VideoState::Received
        );
    }

    #[test]
    fn a_paused_subscription_says_so_and_has_no_subtitle_line() {
        let paused = Facts {
            paused: true,
            video_held: true,
            subtitle_held: true,
            ..facts()
        };
        assert_eq!(video(&paused), VideoState::Paused);
        assert_eq!(subtitle(&paused), None);
    }

    #[test]
    fn the_subtitle_line_waits_until_a_subtitle_is_held() {
        assert_eq!(subtitle(&facts()), Some(SubtitleState::Waiting));
        assert_eq!(
            subtitle(&Facts {
                subtitle_held: true,
                ..facts()
            }),
            Some(SubtitleState::Received)
        );
        // Before the broadcast it waits as well, quietly.
        assert_eq!(
            subtitle(&Facts {
                instant: 2_000,
                ..facts()
            }),
            Some(SubtitleState::Waiting)
        );
        // Creator not decided yet: still waiting.
        assert_eq!(
            subtitle(&Facts {
                subtitles: SubtitleMode::Undecided,
                ..facts()
            }),
            Some(SubtitleState::Waiting)
        );
    }

    #[test]
    fn a_subscription_that_takes_no_subtitles_has_no_subtitle_line() {
        for subtitle_held in [false, true] {
            assert_eq!(
                subtitle(&Facts {
                    subtitles: SubtitleMode::None,
                    subtitle_held,
                    ..facts()
                }),
                None
            );
        }
    }

    #[test]
    fn the_states_the_subtitle_side_will_fill_have_their_names() {
        let json = |state: SubtitleState| serde_json::to_string(&state).unwrap();
        assert_eq!(json(SubtitleState::Downloading), "\"downloading\"");
        assert_eq!(json(SubtitleState::AuthRequired), "\"auth_required\"");
        assert_eq!(
            json(SubtitleState::EpisodeUnconfirmed),
            "\"episode_unconfirmed\""
        );
    }
}
