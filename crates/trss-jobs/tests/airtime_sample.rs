//! The air-time mapping rules ([`trss_jobs::mapping`]) run on what Anissia and
//! AniList really say, once, by hand (ticket 0051's sample):
//!
//! `cargo test -p trss-jobs --test airtime_sample -- --ignored --nocapture`
//!
//! It reads Anissia's recent caption list (pages from 0 until one is empty, at
//! most 5), the weekly schedules for the anime's original titles, and for each
//! anime AniList's entry (found by the original title, only when exactly one
//! entry has that title) with its whole airing schedule. Each line of the
//! recent list is one creator's last episode of an anime, so each line gives a
//! single episode of evidence; the table says what the rules make of it. An
//! AniList entry is taken as the whole season 1 of its anime (no earlier
//! seasons), so a creator who counts on from an earlier cour shows as an offset
//! the rules do not take up.

use std::{collections::BTreeMap, time::Duration};

use trss_anilist::{
    season::fetch_entry,
    title::{decide as match_title, Decision},
    Anilist, AnilistConfig, AnilistError, Entry,
};
use trss_anissia::{observe::CaptionLine, Anissia, AnissiaConfig, LAST_WEEK};
use trss_core::{system_clock, Db};
use trss_jobs::mapping::{decide, offsets, Posted, Season};
use trss_library::seasons::combine::schedule_times;

/// What a line came to.
struct Row {
    anime: String,
    creator: String,
    episode: String,
    updated: String,
    /// The aired season episode the line points to and the offset, or why not.
    pointed: String,
    decision: String,
    class: &'static str,
}

fn when(ms: i64) -> String {
    let secs = ms / 1000 + 9 * 3600;
    let (days, rest) = (secs.div_euclid(86_400), secs.rem_euclid(86_400));
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02} {:02}:{:02}",
        rest / 3600,
        rest % 3600 / 60
    )
}

/// AniList asks to wait: wait that long once and ask again.
async fn patiently<T, F, Fut>(mut ask: F) -> Result<T, AnilistError>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<T, AnilistError>>,
{
    match ask().await {
        Err(AnilistError::Busy { retry_after }) => {
            tokio::time::sleep(retry_after + Duration::from_secs(1)).await;
            ask().await
        }
        other => other,
    }
}

#[tokio::test]
#[ignore = "reaches the real Anissia and AniList"]
async fn what_the_rules_make_of_the_recent_anissia_lines() {
    let db = Db::open(":memory:").await.unwrap();
    let anissia = Anissia::with_defaults(db.clone(), AnissiaConfig::default());
    let anilist = Anilist::new(AnilistConfig::default(), db, system_clock());

    // The recent lines.
    let mut lines: Vec<CaptionLine> = Vec::new();
    for page in 0..5 {
        let answer = anissia.fetch_recent_captions(page, None).await.unwrap();
        if answer.rows == 0 {
            break;
        }
        lines.extend(answer.lines);
    }
    assert!(!lines.is_empty(), "Anissia gave no recent lines");

    // The Korean titles of the anime: the recent list names them, which the
    // client's lines do not keep, so the raw pages are read once more.
    let http = reqwest::Client::builder()
        .user_agent(trss_core::USER_AGENT)
        .build()
        .unwrap();
    let mut subjects: BTreeMap<i64, String> = BTreeMap::new();
    for page in 0..5 {
        tokio::time::sleep(Duration::from_secs(2)).await;
        let bytes = http
            .get(format!(
                "{}/anime/caption/recent/{page}",
                trss_anissia::DEFAULT_URL
            ))
            .send()
            .await
            .unwrap()
            .bytes()
            .await
            .unwrap();
        let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        let content = body["data"]["content"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        if content.is_empty() {
            break;
        }
        for line in content {
            if let (Some(no), Some(subject)) = (line["animeNo"].as_i64(), line["subject"].as_str())
            {
                subjects.insert(no, subject.to_owned());
            }
        }
    }

    // The original titles of the anime: from the schedules (airing now), else
    // from the full list searched by the Korean title.
    let mut titles: BTreeMap<i64, (String, Option<String>)> = BTreeMap::new();
    for week in 0..=LAST_WEEK {
        let schedule = anissia.fetch_schedule(week, None).await.unwrap();
        for anime in schedule {
            titles.insert(anime.anime_no, (anime.subject, anime.original_subject));
        }
    }
    for line in &lines {
        let no = line.anime_no.unwrap();
        if titles.contains_key(&no) {
            continue;
        }
        let Some(subject) = subjects.get(&no) else {
            continue;
        };
        titles.insert(no, (subject.clone(), None));
        for page in 0..3 {
            let Ok(found) = anissia.fetch_anime_page(subject, page, None).await else {
                break;
            };
            if let Some(anime) = found.entries.iter().find(|e| e.anime_no == no) {
                titles.insert(no, (anime.subject.clone(), anime.original_subject.clone()));
                break;
            }
            if found.last {
                break;
            }
        }
    }

    // Each anime's AniList entry, once.
    let mut entries: BTreeMap<i64, Result<Entry, String>> = BTreeMap::new();
    for anime_no in lines.iter().filter_map(|l| l.anime_no) {
        if entries.contains_key(&anime_no) {
            continue;
        }
        let Some((_, Some(original))) = titles.get(&anime_no) else {
            entries.insert(anime_no, Err("원제를 몰라요".to_owned()));
            continue;
        };
        let found = patiently(|| anilist.search_all(original)).await;
        let found = match found {
            Ok(search) => match match_title(original, &search.candidates, search.complete) {
                Decision::Select(c) => Ok(c.id),
                Decision::NoMatch => Err("AniList에 같은 제목이 없어요".to_owned()),
                Decision::Ambiguous => Err("AniList에 같은 제목이 여럿이에요".to_owned()),
                Decision::Incomplete => Err("AniList 검색을 끝까지 읽지 못했어요".to_owned()),
            },
            Err(e) => Err(format!("AniList 검색 실패: {e}")),
        };
        let entry = match found {
            Ok(id) => match patiently(|| fetch_entry(&anilist, id, None, 0)).await {
                Ok(Some(entry)) => Ok(entry),
                Ok(None) => Err("AniList 항목이 없어요".to_owned()),
                Err(e) => Err(format!("AniList 항목 실패: {e}")),
            },
            Err(why) => Err(why),
        };
        entries.insert(anime_no, entry);
    }

    // The rules on each line.
    let mut rows: Vec<Row> = Vec::new();
    for line in &lines {
        let anime_no = line.anime_no.unwrap();
        let anime = titles
            .get(&anime_no)
            .map_or_else(|| anime_no.to_string(), |(subject, _)| subject.clone());
        let mut row = Row {
            anime,
            creator: line.creator.clone(),
            episode: line.episode.clone(),
            updated: line.updated.clone(),
            pointed: String::new(),
            decision: String::new(),
            class: "",
        };
        match &entries[&anime_no] {
            Err(why) => {
                row.decision = why.clone();
                row.class = "AniList 없음";
            }
            Ok(entry) => {
                let schedule = schedule_times(std::slice::from_ref(entry));
                let whole = line
                    .episode
                    .parse::<u32>()
                    .ok()
                    .filter(|n| *n > 0 && !line.episode.starts_with('0'));
                match whole {
                    None => {
                        row.decision = "숫자 회차가 아니라 근거가 아니에요".to_owned();
                        row.class = "회차 아님";
                    }
                    Some(n) => {
                        let posted = [Posted {
                            episode: n,
                            at: line.updated_at,
                        }];
                        let season = Season {
                            number: 1,
                            schedule: &schedule,
                            count: entry.episodes,
                            previous: Some(0),
                        };
                        if let Some(d) = offsets(&posted, &season).get(&n) {
                            let k = i64::from(n) + d;
                            row.pointed = format!("{k}화 방영 뒤, 차이 {d:+}");
                        } else {
                            row.pointed = "방영 창 밖".to_owned();
                        }
                        let decided = decide(&posted, &season);
                        row.class = match (decided.offset, row.pointed.as_str()) {
                            (Some(0), _) => "auto 0",
                            (Some(_), _) => "auto 0 아님",
                            (None, "방영 창 밖") if schedule.is_empty() => "일정 없음",
                            (None, "방영 창 밖") => "근거 없음",
                            (None, _) => "undecided",
                        };
                        row.decision = match decided.offset {
                            Some(d) => format!("auto {d:+} · {}", decided.evidence),
                            None => format!("undecided · {}", decided.evidence),
                        };
                    }
                }
            }
        }
        rows.push(row);
    }

    println!("| 작품 | 제작자 | 회차 | updDt | 가리키는 방영 회차 | 결정 |");
    println!("| --- | --- | --- | --- | --- | --- |");
    for r in &rows {
        let updated =
            trss_anissia::observe::updated_at(&r.updated).map_or_else(|| r.updated.clone(), when);
        println!(
            "| {} | {} | {} | {} | {} | {} |",
            r.anime, r.creator, r.episode, updated, r.pointed, r.decision
        );
    }
    let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
    for r in &rows {
        *counts.entry(r.class).or_default() += 1;
    }
    println!("\n{} lines: {counts:?}", rows.len());
}
