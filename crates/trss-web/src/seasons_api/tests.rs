use std::{collections::BTreeSet, time::Duration};

use axum::{
    body::Body,
    http::{header, Method, Request, StatusCode},
};
use http_body_util::BodyExt;
use serde_json::{json, Value};
use tower::ServiceExt;

use crate::{api, AppState};
use trss_anilist::fake::Fake;
use trss_core::Db;
use trss_library::{
    artwork::{image::samples, AppData, Artwork},
    discovery::{EpisodeFile, FileKind, Scan, ScannedWork, WorkRead},
};

struct Env {
    state: AppState,
    fake: Fake,
    folder: String,
    _dir: tempfile::TempDir,
}

fn episode(season: u32, episode: &str) -> EpisodeFile {
    EpisodeFile {
        path: format!("Season {season:02}/S{season:02}E{episode}.mkv"),
        kind: FileKind::Video,
        season,
        episode: episode.to_owned(),
    }
}

fn work(name: &str, seasons: &[u32], files: Vec<EpisodeFile>) -> WorkRead {
    WorkRead::Read(ScannedWork {
        dir_name: name.to_owned(),
        seasons: BTreeSet::from_iter(seasons.iter().copied()),
        files,
        unrecognized: Vec::new(),
    })
}

/// An entry as AniList answers it.
fn media(id: i64, romaji: &str, native: &str, status: &str, year: i64) -> Value {
    json!({
        "id": id,
        "title": { "romaji": romaji, "english": null, "native": native },
        "synonyms": [],
        "format": "TV", "status": status, "episodes": 12,
        "description": "First<br><br>Second",
        "startDate": { "year": year, "month": 4, "day": null },
        "endDate": { "year": year, "month": 6, "day": null },
        "genres": ["Action"],
        "studios": { "nodes": [{ "name": format!("Studio {id}"), "isAnimationStudio": true }] },
        "relations": { "edges": [] },
        "airingSchedule": { "nodes": [] },
    })
}

async fn env(works: Vec<WorkRead>) -> Env {
    env_in(works, false).await
}

/// [`env`] with an app data folder, so that cover images can be stored.
async fn env_with_covers(works: Vec<WorkRead>) -> Env {
    env_in(works, true).await
}

async fn env_in(works: Vec<WorkRead>, covers: bool) -> Env {
    let dir = tempfile::tempdir().unwrap();
    let db = Db::open(dir.path().join("trss.db")).await.unwrap();
    let fake = Fake::start().await;
    let app_data = covers.then(|| AppData::new(dir.path()));
    let state = AppState::new(db.clone())
        .with_artwork(Artwork::new(db, app_data, fake.config()).with_spacing(Duration::ZERO));
    let (folder, _) = state
        .library
        .add_folder("/c".into(), Scan { works }, 1, &[])
        .await
        .unwrap();
    Env {
        state,
        fake,
        folder: folder.id,
        _dir: dir,
    }
}

impl Env {
    async fn id(&self, name: &str) -> String {
        self.state
            .library
            .works(&self.folder)
            .await
            .unwrap()
            .into_iter()
            .find(|w| w.dir_name == name)
            .unwrap()
            .id
    }

    fn answer(&self, entry: Value) {
        let id = entry["id"].as_i64().unwrap();
        self.fake.state.lock().unwrap().media.insert(id, entry);
    }

    /// Links `ids` as the user would, from the season's current version.
    async fn link(&self, name: &str, season: u32, ids: &[i64]) {
        let work = self.id(name).await;
        let version = self
            .state
            .seasons
            .store
            .link(&work, season)
            .await
            .unwrap()
            .version;
        self.state
            .seasons
            .set_links(&work, season, version, ids.to_vec())
            .await
            .unwrap();
    }
}

async fn call(
    state: &AppState,
    method: Method,
    uri: &str,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let mut request = Request::builder().method(method).uri(uri);
    let body = match body {
        Some(value) => {
            request = request.header(header::CONTENT_TYPE, "application/json");
            Body::from(value.to_string())
        }
        None => Body::empty(),
    };
    let response = api::router()
        .with_state(state.clone())
        .oneshot(request.body(body).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

async fn get(state: &AppState, uri: &str) -> (StatusCode, Value) {
    call(state, Method::GET, uri, None).await
}

async fn post(state: &AppState, uri: &str, body: Value) -> (StatusCode, Value) {
    call(state, Method::POST, uri, Some(body)).await
}

fn names(page: &Value) -> Vec<&str> {
    page["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|w| w["name"].as_str().unwrap())
        .collect()
}

#[tokio::test]
async fn a_season_without_a_link_is_unknown_and_never_filled_from_another_season() {
    let env = env(vec![work(
        "Show",
        &[1, 2],
        vec![episode(1, "01"), episode(2, "01")],
    )])
    .await;
    env.answer(media(1, "Show", "ショー", "FINISHED", 2022));
    env.link("Show", 1, &[1]).await;
    let id = env.id("Show").await;

    let (status, body) = get(&env.state, &format!("/library/works/{id}")).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["native_title"], "ショー");
    let seasons = body["seasons"].as_array().unwrap();
    let (one, two) = (&seasons[0]["info"], &seasons[1]["info"]);
    assert_eq!(one["entries"][0]["id"], 1);
    assert_eq!(one["episodes"], 12);
    assert_eq!(one["studios"], json!(["Studio 1"]));
    assert_eq!(
        one["airing"]["start"],
        json!({ "year": 2022, "month": 4, "day": null })
    );
    assert_eq!(one["origin"], "user");
    assert_eq!(one["anilist_url"], "https://anilist.co/anime/1");
    assert_eq!(one["can_auto"], true);
    // Season 2 has no link: nothing of season 1's, and no AniList page.
    assert_eq!(two["version"], 0);
    assert_eq!(two["entries"], json!([]));
    assert_eq!(two["airing"], Value::Null);
    assert_eq!(two["episodes"], Value::Null);
    assert_eq!(two["studios"], json!([]));
    assert_eq!(two["genres"], json!([]));
    assert_eq!(two["synopsis"], Value::Null);
    assert_eq!(two["anilist_url"], Value::Null);
    assert_eq!(two["pending"], Value::Null);
    assert_eq!(two["can_auto"], false);
}

#[tokio::test]
async fn the_original_title_is_the_first_seasons_first_entrys_and_a_new_work_waits_for_its_search()
{
    let env = env(vec![
        work("Show", &[1, 2], vec![]),
        work("Fresh", &[1], vec![]),
        work("Special", &[0], vec![]),
    ])
    .await;
    env.answer(media(1, "Show", "第一", "FINISHED", 2020));
    env.answer(media(2, "Show 2", "第二", "FINISHED", 2022));
    // Linked to season 2 only: the head's original title is the first season's.
    env.link("Show", 2, &[2]).await;
    let show = env.id("Show").await;
    let (_, body) = get(&env.state, &format!("/library/works/{show}")).await;
    assert_eq!(body["native_title"], Value::Null);
    assert_eq!(body["seasons"][0]["info"]["pending"], "search");

    env.link("Show", 1, &[1, 2]).await;
    let (_, body) = get(&env.state, &format!("/library/works/{show}")).await;
    assert_eq!(body["native_title"], "第一");
    assert_eq!(body["seasons"][0]["info"]["pending"], Value::Null);

    // A new work shows that its search is waiting; a work with only specials has no search.
    let fresh = env.id("Fresh").await;
    let (_, body) = get(&env.state, &format!("/library/works/{fresh}")).await;
    assert_eq!(body["native_title"], Value::Null);
    assert_eq!(body["seasons"][0]["info"]["pending"], "search");
    let special = env.id("Special").await;
    let (_, body) = get(&env.state, &format!("/library/works/{special}")).await;
    assert_eq!(body["seasons"][0]["info"]["pending"], Value::Null);
    assert_eq!(body["seasons"][0]["info"]["can_auto"], false);
}

#[tokio::test]
async fn the_synopsis_is_the_first_entrys_plain_text_in_paragraphs() {
    let env = env(vec![work("Show", &[1], vec![])]).await;
    let mut part1 = media(1, "Show", "ショー", "FINISHED", 2022);
    part1["description"] = json!(
        "Para <i>one</i> &amp; &#039;more&#039;<br>line two<br><br>\
         <script>alert(1)</script><svg onload=alert(2)><br><br>~!Third!~ &lt;b&gt;"
    );
    env.answer(part1);
    env.answer(media(2, "Show 2", "ショー2", "FINISHED", 2023));
    env.link("Show", 1, &[1, 2]).await;
    let id = env.id("Show").await;
    let (_, body) = get(&env.state, &format!("/library/works/{id}")).await;
    assert_eq!(
        body["seasons"][0]["info"]["synopsis"],
        json!(["Para one & 'more'\nline two", "alert(1)", "~!Third!~ <b>"])
    );
    // The second entry's description is not the synopsis.
    env.link("Show", 1, &[2, 1]).await;
    let (_, body) = get(&env.state, &format!("/library/works/{id}")).await;
    assert_eq!(
        body["seasons"][0]["info"]["synopsis"],
        json!(["First", "Second"])
    );
}

#[tokio::test]
async fn episode_air_dates_come_only_from_a_releasing_entry_with_a_schedule() {
    let env = env(vec![work(
        "Show",
        &[1, 2],
        vec![
            episode(1, "01"),
            episode(2, "13"),
            episode(2, "14"),
            episode(2, "015"),
            episode(2, "99"),
            episode(2, "SP"),
        ],
    )])
    .await;
    let mut done = media(31, "Show Part 1", "一", "FINISHED", 2024);
    done["airingSchedule"] = json!({ "nodes": [{ "episode": 1, "airingAt": 1_000 }] });
    let mut airing = media(32, "Show Part 2", "二", "RELEASING", 2025);
    airing["airingSchedule"] = json!({ "nodes": [
        { "episode": 1, "airingAt": 1_700_000_000 },
        { "episode": 2, "airingAt": 1_700_600_000 },
        { "episode": 3, "airingAt": 1_701_200_000 }
    ] });
    env.answer(done);
    env.answer(airing);
    env.link("Show", 1, &[31]).await;
    env.link("Show", 2, &[31, 32]).await;
    let id = env.id("Show").await;
    let (_, body) = get(&env.state, &format!("/library/works/{id}")).await;
    let season1 = &body["seasons"][0]["episodes"];
    assert_eq!(
        season1[0]["air_at"],
        Value::Null,
        "a finished entry has no air dates"
    );
    let season2 = body["seasons"][1]["episodes"].as_array().unwrap();
    let air: Vec<(&str, &Value)> = season2
        .iter()
        .map(|e| (e["episode"].as_str().unwrap(), &e["air_at"]))
        .collect();
    assert_eq!(
        air,
        [
            ("13", &json!(1_700_000_000_000_i64)),
            ("14", &json!(1_700_600_000_000_i64)),
            ("015", &json!(1_701_200_000_000_i64)),
            ("99", &Value::Null),
            ("SP", &Value::Null),
        ]
    );
}

#[tokio::test]
async fn the_library_sorts_by_airing_year_filters_what_is_airing_and_searches_linked_titles() {
    let env = env(vec![
        work("Alpha", &[1], vec![]),
        work("Beta", &[1], vec![]),
        work("Gamma", &[1], vec![]),
        work("Delta", &[1, 2], vec![]),
        work("Epsilon", &[1], vec![]),
    ])
    .await;
    env.answer(media(1, "Alpha Romaji", "アルファ", "FINISHED", 2019));
    env.answer(media(2, "Beta Romaji", "ベータ", "RELEASING", 2024));
    env.answer(media(3, "Delta One", "デルタ", "FINISHED", 2010));
    env.answer(media(4, "Delta Two", "デルタ2", "RELEASING", 2025));
    let mut unknown_year = media(5, "Epsilon Romaji", "イプシロン", "NOT_YET_RELEASED", 2030);
    unknown_year["startDate"] = json!({ "year": null, "month": null, "day": null });
    env.answer(unknown_year);
    env.link("Alpha", 1, &[1]).await;
    env.link("Beta", 1, &[2]).await;
    env.link("Delta", 1, &[3]).await;
    env.link("Delta", 2, &[4]).await;
    env.link("Epsilon", 1, &[5]).await;

    // Latest season's first entry's year, latest first, unknown last (then by title).
    let (_, page) = get(&env.state, "/library/works?sort=year").await;
    assert_eq!(names(&page), ["Delta", "Beta", "Alpha", "Epsilon", "Gamma"]);
    // Airing: the latest season has a releasing entry. Alpha is finished; Epsilon is not yet released.
    let (_, page) = get(&env.state, "/library/works?filter=airing&sort=title").await;
    assert_eq!(names(&page), ["Beta", "Delta"]);
    assert_eq!(
        (page["total"].as_u64(), page["library_count"].as_u64()),
        (Some(2), Some(5))
    );
    // The native, the romaji and the folder name all find the work.
    for (query, expected) in [
        ("ベータ", vec!["Beta"]),
        ("alpha romaji", vec!["Alpha"]),
        ("DELTA TWO", vec!["Delta"]),
        ("デルタ", vec!["Delta"]),
        ("delta", vec!["Delta"]),
        ("イプ", vec!["Epsilon"]),
        ("nothing", vec![]),
    ] {
        let uri = format!(
            "/library/works?q={}",
            url::form_urlencoded::byte_serialize(query.as_bytes()).collect::<String>()
        );
        let (_, page) = get(&env.state, &uri).await;
        assert_eq!(names(&page), expected, "{query}");
    }
    // Sort, filter and search together, and the pages follow the year order.
    let (_, page) = get(&env.state, "/library/works?sort=year&limit=2").await;
    assert_eq!(names(&page), ["Delta", "Beta"]);
    let next = page["next"].as_str().unwrap().to_owned();
    let (_, page) = get(
        &env.state,
        &format!("/library/works?sort=year&limit=2&after={next}"),
    )
    .await;
    assert_eq!(names(&page), ["Alpha", "Epsilon"]);
}

#[tokio::test]
async fn the_latest_local_season_decides_the_year_and_airing_not_an_earlier_one() {
    let env = env(vec![
        work("Delta", &[1, 2], vec![]),
        work("Other", &[1], vec![]),
    ])
    .await;
    env.answer(media(3, "Delta One", "デルタ", "RELEASING", 2010));
    env.answer(media(4, "Other", "別", "FINISHED", 2015));
    // Season 1 is releasing; season 2, the latest, has no link: no year and not airing.
    env.link("Delta", 1, &[3]).await;
    env.link("Other", 1, &[4]).await;
    let (_, page) = get(&env.state, "/library/works?filter=airing").await;
    assert_eq!(names(&page), Vec::<&str>::new());
    let (_, page) = get(&env.state, "/library/works?sort=year").await;
    assert_eq!(names(&page), ["Other", "Delta"]);
}

#[tokio::test]
async fn changing_the_links_answers_the_new_info_and_an_old_version_is_a_409_with_the_current_one()
{
    let env = env(vec![work("Show", &[1, 2], vec![])]).await;
    env.answer(media(1, "A", "あ", "FINISHED", 2022));
    env.answer(media(2, "B", "い", "FINISHED", 2023));
    let id = env.id("Show").await;
    let base = format!("/library/works/{id}/seasons");

    // Season 1 has the first-season row: version 1. Two parts, in order.
    let (status, info) = post(
        &env.state,
        &format!("{base}/1/links"),
        json!({ "version": 1, "anilist_ids": [1, 2] }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{info}");
    assert_eq!(
        (info["version"].as_i64(), info["origin"].as_str()),
        (Some(2), Some("user"))
    );
    assert_eq!(info["episodes"], 24);
    assert_eq!(info["entries"][1]["id"], 2);
    assert_eq!(info["pending"], Value::Null);

    // A change from version 1 changes nothing and gets the current info.
    let (status, error) = post(
        &env.state,
        &format!("{base}/1/links"),
        json!({ "version": 1, "anilist_ids": [2] }),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{error}");
    assert_eq!(error["error"], "conflict");
    assert_eq!(error["current"]["version"], 2);
    assert_eq!(error["current"]["entries"][0]["id"], 1);
    let (_, info) = get(&env.state, &format!("{base}/1/info")).await;
    assert_eq!(info["entries"].as_array().unwrap().len(), 2);

    // Unlinking is a change from the version the screen shows.
    let (_, info) = post(
        &env.state,
        &format!("{base}/1/links"),
        json!({ "version": 2, "anilist_ids": [] }),
    )
    .await;
    assert_eq!(info["entries"], json!([]));
    assert_eq!(info["airing"], Value::Null);
    assert_eq!(info["origin"], "user");
    assert_eq!(info["version"], 3);

    // An ID AniList does not have, a bad body, a season the work lacks.
    let (status, error) = post(
        &env.state,
        &format!("{base}/1/links"),
        json!({ "version": 3, "anilist_ids": [404] }),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{error}");
    assert!(error["message"].as_str().unwrap().contains("AniList"));
    let (status, _) = post(
        &env.state,
        &format!("{base}/1/links"),
        json!({ "version": 3 }),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (status, _) = post(
        &env.state,
        &format!("{base}/1/links"),
        json!({ "version": 3, "anilist_ids": [-1] }),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (status, _) = post(
        &env.state,
        &format!("{base}/9/links"),
        json!({ "version": 0, "anilist_ids": [1] }),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, _) = get(&env.state, "/library/works/nope/seasons/1/info").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn the_next_season_shows_the_sequels_and_links_nothing_until_one_is_confirmed() {
    let env = env(vec![work("Show", &[1, 2, 4], vec![])]).await;
    let mut first = media(1, "A", "あ", "FINISHED", 2022);
    first["relations"] = json!({ "edges": [
        { "relationType": "SEQUEL", "node": { "id": 2, "type": "ANIME", "format": "TV",
            "status": "NOT_YET_RELEASED", "title": { "romaji": "B", "english": null, "native": null },
            "startDate": { "year": 2026, "month": 4, "day": null } } },
        { "relationType": "SEQUEL", "node": { "id": 3, "type": "ANIME", "format": "MOVIE",
            "status": "FINISHED", "title": { "romaji": "The Movie", "english": "Movie", "native": null },
            "startDate": { "year": 2025, "month": 2, "day": 3 } } }
    ] });
    env.answer(first);
    env.answer(media(2, "B", "い", "NOT_YET_RELEASED", 2026));
    env.link("Show", 1, &[1]).await;
    let id = env.id("Show").await;
    let (_, body) = get(&env.state, &format!("/library/works/{id}")).await;
    let seasons = body["seasons"].as_array().unwrap();
    let two = &seasons[1]["info"];
    let offered: Vec<(i64, &str, &str)> = two["suggestions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| {
            (
                s["id"].as_i64().unwrap(),
                s["title"].as_str().unwrap(),
                s["format"].as_str().unwrap(),
            )
        })
        .collect();
    assert_eq!(offered, [(2, "B", "TV"), (3, "Movie", "MOVIE")]);
    assert_eq!(two["suggestions"][0]["start"]["year"], 2026);
    // Until the user confirms, the season is unknown and nothing is waiting.
    assert_eq!(two["entries"], json!([]));
    assert_eq!(two["episodes"], Value::Null);
    assert_eq!(
        (two["version"].clone(), two["pending"].clone()),
        (json!(0), Value::Null)
    );
    // Season 4 follows no local season 3: no suggestion. Season 1 has none either.
    assert_eq!(seasons[2]["info"]["suggestions"], json!([]));
    assert_eq!(seasons[0]["info"]["suggestions"], json!([]));

    // `이 항목이 맞아요`: the single suggestion becomes the link and the offer goes.
    let (status, info) = post(
        &env.state,
        &format!("/library/works/{id}/seasons/2/links"),
        json!({ "version": 0, "anilist_ids": [2] }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{info}");
    assert_eq!(info["entries"][0]["id"], 2);
    assert_eq!(info["suggestions"], json!([]));
    assert_eq!(info["origin"], "user");
}

#[tokio::test]
async fn the_ani_list_link_of_each_season_is_its_own_first_entrys_page() {
    let env = env(vec![work("Show", &[1, 2], vec![])]).await;
    env.answer(media(10, "A", "あ", "FINISHED", 2022));
    env.answer(media(20, "B1", "い", "FINISHED", 2023));
    env.answer(media(21, "B2", "う", "FINISHED", 2023));
    env.link("Show", 1, &[10]).await;
    env.link("Show", 2, &[20, 21]).await;
    let id = env.id("Show").await;
    let (_, body) = get(&env.state, &format!("/library/works/{id}")).await;
    assert_eq!(
        body["seasons"][0]["info"]["anilist_url"],
        "https://anilist.co/anime/10"
    );
    assert_eq!(
        body["seasons"][1]["info"]["anilist_url"],
        "https://anilist.co/anime/20"
    );
    // The cover's AniList ID is not consulted: it differs and nothing here uses it.
    assert!(body["cover_url"].is_null());
}

#[tokio::test]
async fn searching_asks_ani_list_and_defaults_to_the_folder_name() {
    let env = env(vec![work("Lycoris Recoil", &[1], vec![])]).await;
    env.fake.add_search(
        "Lycoris Recoil",
        vec![media(1, "Lycoris Recoil", "リコリス", "FINISHED", 2022)],
        b"",
    );
    let id = env.id("Lycoris Recoil").await;
    let uri = format!("/library/works/{id}/seasons/1/search");
    let (status, found) = post(&env.state, &uri, json!({})).await;
    assert_eq!(status, StatusCode::OK, "{found}");
    assert_eq!(found["items"][0]["id"], 1);
    assert_eq!(found["items"][0]["url"], "https://anilist.co/anime/1");
    assert_eq!(found["has_next"], false);
    assert_eq!(
        env.fake.api_requests().last().unwrap().1["search"],
        "Lycoris Recoil"
    );
    let (_, found) = post(&env.state, &uri, json!({ "q": "  other  ", "page": 2 })).await;
    assert_eq!(found["items"], json!([]));
    assert_eq!(env.fake.api_requests().last().unwrap().1["search"], "other");
    for body in [json!({ "page": 0 }), json!({ "q": "x".repeat(201) })] {
        let (status, _) = post(&env.state, &uri, body).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }
    let (status, _) = post(
        &env.state,
        &format!("/library/works/{id}/seasons/5/search"),
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    // AniList asking to wait is told to the user, not hidden.
    {
        let mut state = env.fake.state.lock().unwrap();
        state.rate_limited = 1;
        state.retry_after = 30;
    }
    let (status, error) = post(&env.state, &uri, json!({})).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(
        error["message"].as_str().unwrap().contains("30초"),
        "{error}"
    );
}

#[tokio::test]
async fn asking_again_unlinks_the_first_season_for_a_new_search_and_refresh_receives_the_entries_again(
) {
    let env = env(vec![work("Show", &[1, 2], vec![])]).await;
    env.answer(media(1, "A", "あ", "FINISHED", 2022));
    env.link("Show", 1, &[1]).await;
    let id = env.id("Show").await;
    let base = format!("/library/works/{id}/seasons");

    let mut changed = media(1, "A", "あ", "FINISHED", 2022);
    changed["description"] = json!("New text");
    env.answer(changed);
    let (status, info) = post(&env.state, &format!("{base}/1/refresh"), json!({})).await;
    assert_eq!(status, StatusCode::OK, "{info}");
    assert_eq!(info["synopsis"], json!(["New text"]));
    assert_eq!(info["version"], 2, "a refresh is no change of the link");

    let (status, info) = post(
        &env.state,
        &format!("{base}/1/auto"),
        json!({ "version": 2 }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{info}");
    assert_eq!(
        (
            info["entries"].clone(),
            info["pending"].clone(),
            info["origin"].clone()
        ),
        (json!([]), json!("search"), json!("auto"))
    );
    assert_eq!(info["version"], 3);
    // Only the first season has an automatic search; an old version is a conflict.
    let (status, _) = post(
        &env.state,
        &format!("{base}/2/auto"),
        json!({ "version": 0 }),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (status, error) = post(
        &env.state,
        &format!("{base}/1/auto"),
        json!({ "version": 2 }),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(error["current"]["pending"], "search");
}

/// An entry the cover can be received from: `media` plus its cover at `/img/<id>.jpg`.
fn with_cover(env: &Env, id: i64, title: &str, image: &[u8]) {
    let mut entry = media(id, title, "ショー", "FINISHED", 2022);
    entry["coverImage"] = env.fake.entry(id, title, &[])["coverImage"].clone();
    env.answer(entry);
    env.fake
        .state
        .lock()
        .unwrap()
        .images
        .insert(format!("{id}.jpg"), image.to_vec());
}

#[tokio::test]
async fn saving_a_link_lets_an_automatic_cover_follow_and_shows_a_failure_where_the_cover_view_reads_it(
) {
    let env = env_with_covers(vec![work("Show", &[1], vec![episode(1, "01")])]).await;
    let id = env.id("Show").await;
    let artwork = format!("/library/works/{id}/artwork");
    let season = format!("/library/works/{id}/seasons/1");
    with_cover(&env, 1, "Show", &samples::jpeg());
    with_cover(&env, 2, "Show (other)", b"not an image");

    // Linking the first season: the link answers at once, the cover follows behind.
    let (status, info) = post(
        &env.state,
        &format!("{season}/links"),
        json!({ "version": 1, "anilist_ids": [1] }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{info}");
    let (_, cover) = get(&env.state, &artwork).await;
    assert_eq!(
        (
            &cover["mode"],
            &cover["anilist_media_id"],
            &cover["pending"]
        ),
        (&json!("auto"), &json!(1), &json!("fetch"))
    );
    assert_eq!(cover["image"], Value::Null);
    let (_, work) = get(&env.state, &format!("/library/works/{id}")).await;
    assert_eq!(
        (&work["cover_url"], &work["cover_pending"]),
        (&Value::Null, &json!(true))
    );

    // Once the worker has received it, the work's page has the cover.
    assert!(env.state.artwork.run_next().await.is_some());
    let (_, cover) = get(&env.state, &artwork).await;
    assert_eq!(cover["pending"], Value::Null);
    assert_eq!(cover["image"]["status"], "available");
    let (_, work) = get(&env.state, &format!("/library/works/{id}")).await;
    assert_eq!(work["cover_pending"], false);
    assert_eq!(work["cover_url"], cover["image"]["url"]);

    // Changing the link to an entry whose image is refused: the link is saved,
    // the old cover stays, and the cover view's state says why.
    let (status, info) = post(
        &env.state,
        &format!("{season}/links"),
        json!({ "version": info["version"], "anilist_ids": [2] }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{info}");
    assert_eq!(info["entries"][0]["id"], 2);
    assert!(env.state.artwork.run_next().await.is_some());
    let (_, after) = get(&env.state, &artwork).await;
    assert_eq!(after["note"]["code"], "rejected");
    assert_eq!(after["pending"], Value::Null);
    assert_eq!(after["image"], cover["image"]);
    assert_eq!(after["mode"], "auto");
    let (_, work) = get(&env.state, &format!("/library/works/{id}")).await;
    assert_eq!(
        (&work["cover_url"], &work["cover_pending"]),
        (&cover["image"]["url"], &json!(false))
    );
}

#[tokio::test]
async fn saving_the_links_of_a_subscribed_creators_season_makes_the_creators_jobs() {
    use trss_collect::store::channels::{ChannelInput, NewSubscription, RuleInput, SubtitleMode};

    let env = env(vec![work("Show", &[1], vec![episode(1, "01")])]).await;
    // A season of 12 that aired weekly from 1_790_000_000 s.
    let mut entry = media(1, "A", "あ", "RELEASING", 2026);
    entry["airingSchedule"] = json!({ "nodes": (1..=12)
        .map(|k| json!({ "episode": k, "airingAt": 1_790_000_000 + (k - 1) * 604_800 }))
        .collect::<Vec<_>>() });
    env.answer(entry);
    let id = env.id("Show").await;
    // A collecting subscription of season 1 that follows 에루샤, whose
    // episodes 1 and 2 were observed an hour after they aired.
    let channel = env
        .state
        .channels
        .create_channel(ChannelInput::new("https://feed.test/rss"))
        .await
        .unwrap();
    let rule = env
        .state
        .channels
        .create_subscription_rule(
            &channel.id,
            RuleInput {
                r#match: Some("Show".into()),
                directory: "Show".into(),
                ..RuleInput::default()
            },
            NewSubscription {
                anime: trss_anissia::Anime {
                    anime_no: 3441,
                    subject: "작품".into(),
                    original_subject: None,
                    week: 3,
                    air_time: None,
                    start_date: None,
                    end_date: None,
                    status: "ON".into(),
                    fetched_at: 1,
                },
                subtitles: SubtitleMode::Follow,
                creator: Some("에루샤".into()),
                subscribed_at: 1,
            },
        )
        .await
        .unwrap();
    env.state
        .channels
        .link_season(&rule.id, &format!("{id}:1"))
        .await
        .unwrap();
    env.state
        .db()
        .run::<_, trss_core::DbError, _>(|c| {
            c.execute_batch(
                "INSERT INTO subtitle_sources (id, anime_no, creator_name, created_at)
                     VALUES ('src-a', 3441, '에루샤', 1);
                 INSERT INTO caption_observations
                     (source_id, post_url, episode, updated, updated_at, first_seen_at)
                 VALUES ('src-a', 'https://blog.test/ep1', '1', 'x', 1790003600000, 2),
                        ('src-a', 'https://blog.test/ep2', '2', 'x', 1790608400000, 2);",
            )?;
            Ok(())
        })
        .await
        .unwrap();
    // Without the season's schedule, nothing is decided or received.
    assert!(env.state.follow.evaluate(1).await.unwrap().is_empty());

    let version = env.state.seasons.store.link(&id, 1).await.unwrap().version;
    let (status, info) = post(
        &env.state,
        &format!("/library/works/{id}/seasons/1/links"),
        json!({ "version": version, "anilist_ids": [1] }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{info}");
    let open = env.state.jobs.open_jobs().await.unwrap();
    assert_eq!(open.len(), 2);
    assert!(open.iter().all(|job| job.origin == trss_jobs::AUTO));
}
