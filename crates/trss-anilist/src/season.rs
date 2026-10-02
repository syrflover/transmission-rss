//! The AniList question season info asks: one anime entry with what the screen
//! shows (dates, episode count, studios, genres, description, the airing
//! schedule) and the entries that follow it. It goes through the artwork
//! client ([`Anilist`]), so it keeps the same pace, `429` wait and timeouts as
//! every other request of the app.

use std::time::Duration;

use serde::Deserialize;
use serde_json::json;

use trss_core::Millis;

use crate::{Airing, Anilist, AnilistError, Entry, FuzzyDate, Sequel};

/// The most airing schedule entries asked for (AniList's page size caps it).
const SCHEDULE_PAGE: u32 = 50;

fn entry_query() -> String {
    format!(
        "query ($id: Int) {{ Media(id: $id, type: ANIME) {{ \
           id title {{ romaji english native }} synonyms format status episodes description(asHtml: false) \
           startDate {{ year month day }} endDate {{ year month day }} genres \
           studios(isMain: true) {{ nodes {{ name isAnimationStudio }} }} \
           relations {{ edges {{ relationType node {{ id type format status \
             title {{ romaji english native }} startDate {{ year month day }} }} }} }} \
           airingSchedule(perPage: {SCHEDULE_PAGE}) {{ nodes {{ episode airingAt }} }} }} }}"
    )
}

#[derive(Deserialize)]
struct Title {
    romaji: Option<String>,
    english: Option<String>,
    native: Option<String>,
}

#[derive(Deserialize)]
struct Date {
    year: Option<i64>,
    month: Option<i64>,
    day: Option<i64>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Studio {
    name: Option<String>,
    is_animation_studio: Option<bool>,
}

#[derive(Deserialize)]
struct Studios {
    nodes: Option<Vec<Option<Studio>>>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RelationNode {
    id: i64,
    #[serde(rename = "type")]
    kind: Option<String>,
    format: Option<String>,
    status: Option<String>,
    title: Option<Title>,
    start_date: Option<Date>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RelationEdge {
    relation_type: Option<String>,
    node: Option<RelationNode>,
}

#[derive(Deserialize)]
struct Relations {
    edges: Option<Vec<Option<RelationEdge>>>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct AiringNode {
    episode: Option<i64>,
    airing_at: Option<i64>,
}

#[derive(Deserialize)]
struct AiringSchedule {
    nodes: Option<Vec<Option<AiringNode>>>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Media {
    id: i64,
    title: Option<Title>,
    format: Option<String>,
    status: Option<String>,
    episodes: Option<i64>,
    description: Option<String>,
    start_date: Option<Date>,
    end_date: Option<Date>,
    genres: Option<Vec<Option<String>>>,
    synonyms: Option<Vec<Option<String>>>,
    studios: Option<Studios>,
    relations: Option<Relations>,
    airing_schedule: Option<AiringSchedule>,
}

#[derive(Deserialize)]
struct Data {
    #[serde(rename = "Media")]
    media: Option<Media>,
}

fn text(value: Option<String>) -> Option<String> {
    value.map(|v| v.trim().to_owned()).filter(|v| !v.is_empty())
}

/// AniList's fuzzy date, keeping only what makes sense: a month needs a year and
/// a day needs a month.
fn date(value: Option<Date>) -> FuzzyDate {
    let Some(value) = value else {
        return FuzzyDate::default();
    };
    let year = value
        .year
        .filter(|y| (1800..=2300).contains(y))
        .map(|y| y as i32);
    let month = year.and(
        value
            .month
            .filter(|m| (1..=12).contains(m))
            .map(|m| m as u32),
    );
    let day = month.and(value.day.filter(|d| (1..=31).contains(d)).map(|d| d as u32));
    FuzzyDate { year, month, day }
}

fn unique(values: impl Iterator<Item = String>) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for value in values {
        if !out.contains(&value) {
            out.push(value);
        }
    }
    out
}

/// Whether `title` has a Hangul syllable or jamo in it.
fn has_hangul(title: &str) -> bool {
    title.chars().any(|c| {
        matches!(c, '\u{AC00}'..='\u{D7A3}' | '\u{1100}'..='\u{11FF}' | '\u{3130}'..='\u{318F}')
    })
}

fn entry_of(media: Media, fetched_at: Millis) -> Entry {
    let title = media.title.unwrap_or(Title {
        romaji: None,
        english: None,
        native: None,
    });
    let studios = media
        .studios
        .and_then(|s| s.nodes)
        .unwrap_or_default()
        .into_iter()
        .flatten()
        .filter(|s| s.is_animation_studio == Some(true))
        .filter_map(|s| text(s.name));
    let genres = media.genres.unwrap_or_default().into_iter().flatten();
    let sequels = media
        .relations
        .and_then(|r| r.edges)
        .unwrap_or_default()
        .into_iter()
        .flatten()
        .filter(|edge| edge.relation_type.as_deref() == Some("SEQUEL"))
        .filter_map(|edge| edge.node)
        .filter(|node| node.id > 0 && node.kind.as_deref() == Some("ANIME"))
        .map(|node| {
            let title = node.title.unwrap_or(Title {
                romaji: None,
                english: None,
                native: None,
            });
            Sequel {
                id: node.id,
                romaji: text(title.romaji),
                english: text(title.english),
                native: text(title.native),
                format: text(node.format),
                status: text(node.status),
                start: date(node.start_date),
            }
        });
    let mut sequels_out: Vec<Sequel> = Vec::new();
    for sequel in sequels {
        if sequels_out.iter().all(|s| s.id != sequel.id) {
            sequels_out.push(sequel);
        }
    }
    let mut airing: Vec<Airing> = media
        .airing_schedule
        .and_then(|s| s.nodes)
        .unwrap_or_default()
        .into_iter()
        .flatten()
        .filter_map(|node| {
            let episode = u32::try_from(node.episode?).ok().filter(|e| *e > 0)?;
            let at = node.airing_at.filter(|at| *at > 0)?;
            Some(Airing { episode, at })
        })
        .collect();
    airing.sort_by_key(|a| a.episode);
    airing.dedup_by_key(|a| a.episode);
    Entry {
        id: media.id,
        romaji: text(title.romaji),
        english: text(title.english),
        native: text(title.native),
        format: text(media.format),
        status: text(media.status),
        episodes: media
            .episodes
            .and_then(|n| u32::try_from(n).ok())
            .filter(|n| *n > 0),
        start: date(media.start_date),
        end: date(media.end_date),
        studios: unique(studios),
        genres: unique(genres.filter_map(|g| text(Some(g)))),
        korean_titles: unique(
            media
                .synonyms
                .unwrap_or_default()
                .into_iter()
                .flatten()
                .filter_map(|s| text(Some(s)))
                .filter(|s| has_hangul(s)),
        ),
        description: text(media.description),
        airing,
        sequels: sequels_out,
        fetched_at,
    }
}

/// The anime entry `id` as AniList describes it now, received at `now`;
/// `None` when AniList has none. `max_wait` is as in [`Anilist::media`].
pub async fn fetch_entry(
    anilist: &Anilist,
    id: i64,
    max_wait: Option<Duration>,
    now: Millis,
) -> Result<Option<Entry>, AnilistError> {
    let data: Option<Data> = anilist
        .post(entry_query(), json!({ "id": id }), max_wait)
        .await?;
    Ok(data
        .and_then(|d| d.media)
        .filter(|m| m.id == id)
        .map(|m| entry_of(m, now)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn media(value: serde_json::Value) -> Entry {
        let media: Media = serde_json::from_value(value).unwrap();
        entry_of(media, 5)
    }

    #[test]
    fn only_the_synonyms_with_hangul_are_kept_as_the_entrys_korean_titles() {
        let entry = media(json!({
            "id": 9,
            "title": { "romaji": "Bocchi the Rock!", "english": null, "native": "ぼっち・ざ・ろっく！" },
            "synonyms": [
                "봇치 더 록!", "Bocchi", "ぼっち", null, "  ", "봇치 더 록!", "외톨이 THE ROCK!",
                "孤独摇滚", "ㅂㅊ"
            ]
        }));
        assert_eq!(
            entry.korean_titles,
            ["봇치 더 록!", "외톨이 THE ROCK!", "ㅂㅊ"]
        );
        // An answer with no synonyms has none.
        let none = media(json!({ "id": 10, "title": { "romaji": "X" } }));
        assert!(none.korean_titles.is_empty());
        let null = media(json!({ "id": 11, "synonyms": null }));
        assert!(null.korean_titles.is_empty());
    }

    #[test]
    fn the_question_asks_for_the_synonyms() {
        assert!(entry_query().contains("synonyms"));
    }

    #[test]
    fn an_answer_becomes_an_entry_with_only_what_the_screen_may_show() {
        let entry = media(json!({
            "id": 7,
            "title": { "romaji": " Lycoris Recoil ", "english": "", "native": "リコリス・リコイル" },
            "format": "TV", "status": "FINISHED", "episodes": 13,
            "description": "A<br>B",
            "startDate": { "year": 2022, "month": 7, "day": 2 },
            "endDate": { "year": 2022, "month": null, "day": 5 },
            "genres": ["Action", null, "Action", "Comedy"],
            "studios": { "nodes": [
                { "name": "A-1 Pictures", "isAnimationStudio": true },
                { "name": "Aniplex", "isAnimationStudio": false },
                { "name": "A-1 Pictures", "isAnimationStudio": true },
                null
            ] },
            "relations": { "edges": [
                { "relationType": "PREQUEL", "node": { "id": 1, "type": "ANIME" } },
                { "relationType": "SEQUEL", "node": { "id": 8, "type": "ANIME", "format": "TV",
                    "status": "NOT_YET_RELEASED", "title": { "romaji": "Next", "english": null, "native": null },
                    "startDate": { "year": 2026, "month": 4, "day": null } } },
                { "relationType": "SEQUEL", "node": { "id": 9, "type": "MANGA" } },
                { "relationType": "SEQUEL", "node": { "id": 8, "type": "ANIME" } },
                null
            ] },
            "airingSchedule": { "nodes": [
                { "episode": 2, "airingAt": 200 }, { "episode": 1, "airingAt": 100 },
                { "episode": 0, "airingAt": 50 }, { "episode": 3, "airingAt": null }
            ] }
        }));
        assert_eq!(entry.id, 7);
        assert_eq!(entry.romaji.as_deref(), Some("Lycoris Recoil"));
        assert_eq!(entry.english, None);
        assert_eq!(entry.episodes, Some(13));
        assert_eq!(entry.studios, ["A-1 Pictures"]);
        assert_eq!(entry.genres, ["Action", "Comedy"]);
        // A day with no month is dropped.
        assert_eq!(
            entry.end,
            FuzzyDate {
                year: Some(2022),
                month: None,
                day: None
            }
        );
        assert_eq!(
            entry.start,
            FuzzyDate {
                year: Some(2022),
                month: Some(7),
                day: Some(2)
            }
        );
        assert_eq!(entry.sequels.len(), 1);
        assert_eq!(entry.sequels[0].id, 8);
        assert_eq!(
            entry.sequels[0].start,
            FuzzyDate {
                year: Some(2026),
                month: Some(4),
                day: None
            }
        );
        assert_eq!(
            entry.airing,
            [
                Airing {
                    episode: 1,
                    at: 100
                },
                Airing {
                    episode: 2,
                    at: 200
                }
            ]
        );
        assert_eq!(entry.description.as_deref(), Some("A<br>B"));
        assert_eq!(entry.fetched_at, 5);
    }

    #[test]
    fn missing_and_nonsense_values_are_unknown() {
        let entry = media(json!({
            "id": 3, "title": null, "episodes": 0,
            "startDate": { "year": 99999, "month": 3, "day": 4 },
            "studios": null, "genres": null, "relations": null, "airingSchedule": null
        }));
        assert_eq!(entry.episodes, None);
        assert_eq!(entry.start, FuzzyDate::default());
        assert!(entry.studios.is_empty() && entry.genres.is_empty() && entry.airing.is_empty());
        assert_eq!(entry.display_title(), "");
    }
}
