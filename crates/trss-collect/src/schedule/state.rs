//! The status lines of a card on the weekly schedule
//! (`docs/requirements/web-app.md`, 이번 주 편성): one for the video and, unless the
//! subscription takes no subtitles, one for the subtitle. Also Anissia's word
//! on the anime's run ([`run_state`]), which the subscription cards and the
//! schedule picker show (`docs/requirements/collection.md`, 방영작 구독).

use serde::Serialize;

use crate::store::channels::SubtitleMode;
use trss_anissia::{calendar::PartialDate, slot::run_over, Anime};
use trss_core::Millis;

/// The video line of a card.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
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
    /// `결방`: Anissia marks the anime `OFF` (a break or a long hiatus), so
    /// there is no episode to wait for. Quiet. It stands in place of both the
    /// video and the subtitle line.
    Off,
}

/// The subtitle line of a card.
///
/// `Downloading`, `AuthRequired` and `EpisodeUnconfirmed` are the states the
/// subtitle side will fill (`자막 받는 중`, `인증 필요`, `회차 확인 필요`);
/// nothing produces them yet, so the weekly schedule never sends them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
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
    /// Anissia's snapshot of the anime says `OFF` ([`is_off`]).
    pub off: bool,
    pub subtitles: SubtitleMode,
    /// The library has the episode's video / a subtitle for it.
    pub video_held: bool,
    pub subtitle_held: bool,
    /// Transmission is downloading a torrent of the episode.
    pub downloading: bool,
}

/// Whether Anissia calls the anime `OFF` (`결방`).
///
/// Only a snapshot Anissia really gave counts: the stand-in an import keeps
/// while Anissia could not be asked (`fetched_at` 0, status `OFF`) is not
/// Anissia's word, and an entry with no status was read as `ON`.
pub fn is_off(anime: &Anime) -> bool {
    anime.fetched_at > 0 && anime.status.eq_ignore_ascii_case("OFF")
}

/// Where an anime's run is, as the subscription cards and the schedule picker
/// say it (`표: Anissia의 방영 상태` in `docs/requirements/web-app.md`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum RunState {
    /// It has not started: the start date is still to come. The screens show
    /// the start date instead of a state.
    Upcoming,
    /// `방영 중`.
    On,
    /// `결방`: Anissia marks it `OFF` ([`is_off`]).
    Off,
    /// `종영`: Anissia calls it `END`, or the run is over by the rule the
    /// weekly schedule drops its card by ([`run_over`]).
    End,
}

/// The run state of `anime` on the Seoul day `today`. `unlisted` is whether the
/// worker's daily refresh found Anissia no longer lists it. An end that has
/// come wins over everything, then a start still to come, then `OFF`. The
/// stand-in an import keeps while Anissia could not be asked (`fetched_at` 0)
/// is not Anissia's word and has no state.
pub fn run_state(anime: &Anime, unlisted: bool, today: i64) -> Option<RunState> {
    if anime.fetched_at <= 0 {
        return None;
    }
    let ended =
        anime.status.eq_ignore_ascii_case("END") || run_over(anime, unlisted, today).is_some();
    let starts_later = anime
        .start_date
        .as_deref()
        .and_then(PartialDate::parse)
        .is_some_and(|start| start.is_after(today));
    Some(if ended {
        RunState::End
    } else if starts_later {
        RunState::Upcoming
    } else if is_off(anime) {
        RunState::Off
    } else {
        RunState::On
    })
}

/// The video line. A paused subscription says so instead of anything else:
/// it does not receive, whatever the library has. Next comes `결방`, which
/// leaves nothing to receive this week.
pub fn video(facts: &Facts) -> VideoState {
    if facts.paused {
        VideoState::Paused
    } else if facts.off {
        VideoState::Off
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

/// The first moment after `facts.now` at which [`video`] says otherwise than it
/// does now, because time passes and nothing is written: `방영 전` becomes `영상
/// 대기` when the episode airs. Nothing is written when that happens, so the
/// server announces it (`trss::deadlines`). The other lines change only when
/// something is written, or at the deadlines of what they are made of (the
/// cycle's and the look at Transmission's).
pub fn video_changes_at(facts: &Facts) -> Option<Millis> {
    (video(facts) == VideoState::Upcoming).then_some(facts.instant)
}

/// The subtitle line, or `None` when the card has none: the subscription takes
/// no subtitles (`받지 않음`), or its video is off, which leaves subtitles
/// nothing to wait for (`자막 받기` is disabled then), or the anime is `결방`.
pub fn subtitle(facts: &Facts) -> Option<SubtitleState> {
    if facts.paused || facts.off || facts.subtitles == SubtitleMode::None {
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
            off: false,
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
    fn an_upcoming_video_line_changes_when_the_episode_airs_and_no_other_does() {
        let upcoming = Facts {
            instant: 2_000,
            ..facts()
        };
        assert_eq!(video(&upcoming), VideoState::Upcoming);
        assert_eq!(video_changes_at(&upcoming), Some(2_000));
        // Just before, at and after the air time, by the rule itself.
        for (now, line) in [
            (1_999, VideoState::Upcoming),
            (2_000, VideoState::Waiting),
            (2_001, VideoState::Waiting),
        ] {
            assert_eq!(video(&Facts { now, ..upcoming }), line, "at {now}");
        }
        // Asked at the moment it comes, or after, there is none ahead.
        assert_eq!(
            video_changes_at(&Facts {
                now: 2_000,
                ..upcoming
            }),
            None
        );
        assert_eq!(
            video_changes_at(&Facts {
                now: 2_001,
                ..upcoming
            }),
            None
        );
        // The lines that are not `방영 전` do not change with the clock alone.
        for other in [
            Facts {
                paused: true,
                ..upcoming
            },
            Facts {
                off: true,
                ..upcoming
            },
            Facts {
                video_held: true,
                ..upcoming
            },
            Facts {
                downloading: true,
                ..upcoming
            },
            facts(),
        ] {
            assert_eq!(video_changes_at(&other), None, "{:?}", video(&other));
        }
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
    fn an_anime_anissia_calls_off_is_off_in_place_of_both_lines() {
        for (video_held, subtitle_held, downloading, instant) in [
            (false, false, false, 500),
            (true, true, false, 500),
            (false, false, true, 500),
            (false, false, false, 2_000),
        ] {
            let off = Facts {
                off: true,
                video_held,
                subtitle_held,
                downloading,
                instant,
                ..facts()
            };
            assert_eq!(video(&off), VideoState::Off);
            assert_eq!(subtitle(&off), None);
        }
        assert_eq!(serde_json::to_string(&VideoState::Off).unwrap(), "\"off\"");
    }

    #[test]
    fn a_paused_subscription_of_an_off_anime_still_says_paused() {
        let both = Facts {
            paused: true,
            off: true,
            ..facts()
        };
        assert_eq!(video(&both), VideoState::Paused);
        assert_eq!(subtitle(&both), None);
    }

    fn snapshot(status: &str, fetched_at: Millis) -> Anime {
        Anime {
            anime_no: 1,
            subject: "작품".into(),
            original_subject: None,
            week: 3,
            air_time: None,
            start_date: None,
            end_date: None,
            status: status.into(),
            fetched_at,
        }
    }

    #[test]
    fn only_a_status_anissia_gave_as_off_counts() {
        assert!(is_off(&snapshot("OFF", 5)));
        assert!(is_off(&snapshot("off", 5)));
        assert!(!is_off(&snapshot("ON", 5)));
        // The stand-in kept for an anime Anissia could not be asked about.
        assert!(!is_off(&snapshot("OFF", 0)));
    }

    #[test]
    fn the_run_state_is_the_end_then_a_start_to_come_then_off() {
        use trss_core::calendar::days_from_civil;
        let today = days_from_civil(2026, 10, 13);
        let anime = |status: &str, start: Option<&str>, end: Option<&str>| Anime {
            start_date: start.map(str::to_owned),
            end_date: end.map(str::to_owned),
            ..snapshot(status, 5)
        };
        let state = |a: &Anime| run_state(a, false, today);

        assert_eq!(
            state(&anime("ON", Some("2026-10-01"), Some("2026-12-24"))),
            Some(RunState::On)
        );
        assert_eq!(
            state(&anime("OFF", Some("2026-10-01"), Some("2026-12-24"))),
            Some(RunState::Off)
        );
        // On its end date it still airs; the day after, it has ended, `OFF` or not.
        assert_eq!(
            state(&anime("ON", None, Some("2026-10-13"))),
            Some(RunState::On)
        );
        assert_eq!(
            state(&anime("ON", None, Some("2026-10-12"))),
            Some(RunState::End)
        );
        assert_eq!(
            state(&anime("OFF", None, Some("2026-10-12"))),
            Some(RunState::End)
        );
        // Anissia's own `END`, from its full list, even before the end date.
        assert_eq!(
            state(&anime("END", None, Some("2026-12-24"))),
            Some(RunState::End)
        );
        // Without an end date only the daily refresh's listing ends it.
        assert_eq!(state(&anime("ON", None, None)), Some(RunState::On));
        assert_eq!(
            run_state(&anime("ON", None, None), true, today),
            Some(RunState::End)
        );
        // Listed with an end date to come, being unlisted does not end it.
        assert_eq!(
            run_state(&anime("ON", None, Some("2026-12-24")), true, today),
            Some(RunState::On)
        );
        // A start to come, by the day or the month, is before any `OFF`.
        assert_eq!(
            state(&anime("ON", Some("2026-10-14"), None)),
            Some(RunState::Upcoming)
        );
        assert_eq!(
            state(&anime("OFF", Some("2026-11"), None)),
            Some(RunState::Upcoming)
        );
        assert_eq!(
            state(&anime("ON", Some("2026-10"), None)),
            Some(RunState::On)
        );
        // The stand-in kept while Anissia could not be asked has no state.
        assert_eq!(run_state(&snapshot("OFF", 0), false, today), None);
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
