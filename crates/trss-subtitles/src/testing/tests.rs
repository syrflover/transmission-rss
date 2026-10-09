use sha2::{Digest, Sha256};
use url::Url;

use super::*;
use crate::{verify, FailureKind, FileInfo, Opened, Source};

const SRT: &[u8] = b"1\n00:00:01,000 --> 00:00:02,000\nHi\n";

async fn bytes_of(source: &Source, post: &Url, file: &crate::PostFile) -> (Option<u64>, Vec<u8>) {
    let mut fetch = source.fetch(post, file).await.unwrap();
    let mut bytes = Vec::new();
    while let Some(piece) = fetch.chunk().await.unwrap() {
        bytes.extend_from_slice(&piece);
    }
    (fetch.expected_size, bytes)
}

async fn files(source: &Source, post: &Url) -> Vec<crate::PostFile> {
    match source.open(post, "24").await.unwrap() {
        Opened::Files(files) => files,
        other => panic!("files: {other:?}"),
    }
}

#[tokio::test]
async fn a_zip_comes_whole_with_no_cookie_or_referer() {
    let server = SourceServer::start().await;
    let zip = verify::zip_of(&[("Seihantai - 24.srt", SRT)]);
    server.post(
        "sumomomo",
        492,
        vec![PostAnswer::Files(vec![spec(
            "z1",
            "Seihantai - 24.zip",
            "0.01MB",
        )])],
    );
    server.file("z1", vec![FileAnswer::Bytes(zip.clone())]);
    let source = Source::Tistory(server.source());
    let post = Url::parse(&server.post_url("sumomomo", 492)).unwrap();

    let files = files(&source, &post).await;
    assert_eq!(files[0].name, "Seihantai - 24.zip");
    let (expected, bytes) = bytes_of(&source, &post, &files[0]).await;
    assert_eq!(expected, Some(zip.len() as u64));
    assert_eq!(Sha256::digest(&bytes), Sha256::digest(&zip));
    assert!(server.seen().iter().all(|s| !s.cookie && !s.referer));
}

#[tokio::test]
async fn a_refused_address_is_read_again_once_then_expired_or_missing() {
    let server = SourceServer::start().await;
    let source = Source::Tistory(server.source());
    let post = Url::parse(&server.post_url("blog", 1)).unwrap();
    let a = spec("a", "a.zip", "1KB");

    // Refused once: the post is read again and the new address works.
    server.post("blog", 1, vec![PostAnswer::Files(vec![a.clone()])]);
    server.file(
        "a",
        vec![FileAnswer::Refused, FileAnswer::Bytes(b"PK".to_vec())],
    );
    let first = files(&source, &post).await;
    let (_, bytes) = bytes_of(&source, &post, &first[0]).await;
    assert_eq!(bytes, b"PK");
    let posts = server
        .seen()
        .iter()
        .filter(|s| s.host.ends_with("tistory.com"))
        .count();
    assert_eq!(posts, 2);

    // Refused again: expired, with what answered.
    server.file("a", vec![FileAnswer::Refused]);
    let first = files(&source, &post).await;
    let failure = source.fetch(&post, &first[0]).await.err().unwrap();
    assert_eq!(failure.kind, FailureKind::Expired);
    assert_eq!(failure.status, Some(404));
    assert_eq!(failure.content_type.as_deref(), Some("text/html"));
    assert_eq!(failure.size, Some(150));

    // Gone from the post when it is read again: missing.
    server.post(
        "blog",
        1,
        vec![
            PostAnswer::Files(vec![a]),
            PostAnswer::Files(vec![spec("b", "b.zip", "1KB")]),
        ],
    );
    let first = files(&source, &post).await;
    let failure = source.fetch(&post, &first[0]).await.err().unwrap();
    assert_eq!(failure.kind, FailureKind::Missing);
}

#[tokio::test]
async fn a_missing_post_a_failing_site_and_a_drive_post_each_say_so() {
    let server = SourceServer::start().await;
    let source = Source::Tistory(server.source());
    let url = |n| Url::parse(&server.post_url("blog", n)).unwrap();
    server.post("blog", 2, vec![PostAnswer::Status(503)]);
    server.post(
        "blog",
        3,
        vec![PostAnswer::Page(drive_page(
            "1zqWESZSANz8Uj2aaW9u1oSHKurLuTqQw",
        ))],
    );

    let missing = source.open(&url(99999), "1").await.unwrap_err();
    assert_eq!(
        (missing.kind, missing.status, missing.size),
        (FailureKind::Missing, Some(404), Some(1952))
    );
    assert_eq!(
        source.open(&url(2), "1").await.unwrap_err().kind,
        FailureKind::Network
    );
    // A Drive link in the body: the file, from Drive, under the name
    // Drive gives it.
    let ass = crate::fake::ass("FX Senshi Kurumi 01");
    server.drive(
        "1zqWESZSANz8Uj2aaW9u1oSHKurLuTqQw",
        vec![DriveAnswer::File {
            name: "FX 전사 쿠루미 01.ass".into(),
            bytes: ass.clone(),
        }],
    );
    let drive = files(&source, &url(3)).await;
    assert_eq!(drive.len(), 1);
    assert_eq!(drive[0].key, "drive:1zqWESZSANz8Uj2aaW9u1oSHKurLuTqQw");
    let mut fetch = source.fetch(&url(3), &drive[0]).await.unwrap();
    assert_eq!(fetch.name.as_deref(), Some("FX 전사 쿠루미 01.ass"));
    assert_eq!(fetch.expected_size, Some(ass.len() as u64));
    let mut bytes = Vec::new();
    while let Some(piece) = fetch.chunk().await.unwrap() {
        bytes.extend_from_slice(&piece);
    }
    assert_eq!(bytes, ass);
    assert!(server.seen().iter().all(|s| !s.cookie && !s.referer));
}

/// A Blogger post of C소라's shape: the fonts, then 13–24화.
fn csora(server: &SourceServer) -> Url {
    let ids: Vec<(String, String)> =
        std::iter::once(("폰트".to_owned(), "1font0000000".to_owned()))
            .chain((13..=24).map(|n| (format!("{n}화"), format!("1episode{n:04}"))))
            .collect();
    let links: Vec<(String, String)> = ids
        .iter()
        .map(|(w, id)| (w.clone(), drive_link(id)))
        .collect();
    let links: Vec<(&str, &str)> = links
        .iter()
        .map(|(w, h)| (w.as_str(), h.as_str()))
        .collect();
    server.blogger_post(
        "csora556",
        "2026/07/2.html",
        vec![PostAnswer::Page(blogger_page(&links))],
    );
    Url::parse(&server.blogger_url("csora556", "2026/07/2.html")).unwrap()
}

#[tokio::test]
async fn a_blogger_post_offers_its_episodes_file_and_fonts_from_drive() {
    let server = SourceServer::start().await;
    let source = Source::Blogger(server.blogger());
    let post = csora(&server);
    let chosen = match source.open(&post, "24").await.unwrap() {
        Opened::Files(files) => files,
        other => panic!("files: {other:?}"),
    };
    let keys: Vec<&str> = chosen.iter().map(|f| f.key.as_str()).collect();
    assert_eq!(keys, ["drive:1font0000000", "drive:1episode0024"]);
    assert_eq!(
        chosen[1].snapshot.entries(),
        [(
            crate::blogger::POST_MODIFIED.to_owned(),
            BLOGGER_MODIFIED.to_owned()
        )]
    );
    let ass = crate::fake::ass("Seihantai 24");
    server.drive(
        "1episode0024",
        vec![DriveAnswer::File {
            name: "Seihantai 24.ass".into(),
            bytes: ass.clone(),
        }],
    );
    let (expected, bytes) = bytes_of(&source, &post, &chosen[1]).await;
    assert_eq!((expected, bytes), (Some(ass.len() as u64), ass.clone()));
    let fetch = source.fetch(&post, &chosen[1]).await.unwrap();
    assert_eq!(
        fetch.snapshot.entries(),
        [
            (
                crate::http::LAST_MODIFIED.to_owned(),
                DRIVE_MODIFIED.to_owned()
            ),
            (
                crate::drive::CONTENT_LENGTH.to_owned(),
                ass.len().to_string()
            ),
        ]
    );
    // A second file from Drive, after Drive set its cookie twice.
    let font = b"OTTO font".to_vec();
    server.drive(
        "1font0000000",
        vec![DriveAnswer::File {
            name: "fonts.zip".into(),
            bytes: font.clone(),
        }],
    );
    let (_, bytes) = bytes_of(&source, &post, &chosen[0]).await;
    assert_eq!(bytes, font);
    assert_eq!(server.drive_asked("1font0000000"), 1);
    // Only the chosen files were asked for, and no other episode's; none
    // of the requests carried Drive's cookie or a `Referer`.
    assert_eq!(server.drive_asked("1episode0023"), 0);
    assert!(server.seen().iter().all(|s| !s.cookie && !s.referer));

    let failure = source.open(&post, "12").await.unwrap_err();
    assert_eq!(failure.kind, FailureKind::Changed);
    assert_eq!(failure.reason, "게시물에 12화 파일이 없어요");
}

#[tokio::test]
async fn a_page_from_drive_instead_of_the_file_is_a_classified_failure() {
    let server = SourceServer::start().await;
    let source = Source::Blogger(server.blogger());
    let post = csora(&server);
    let Opened::Files(chosen) = source.open(&post, "13").await.unwrap() else {
        panic!("files");
    };
    let file = &chosen[1];
    let cases = [
        (
            DriveAnswer::Confirm,
            FailureKind::NotAFile,
            Some(200),
            "확인 페이지",
        ),
        (DriveAnswer::Quota, FailureKind::NotAFile, Some(200), "한도"),
        (
            DriveAnswer::Missing,
            FailureKind::Missing,
            Some(404),
            "없어요",
        ),
        (
            DriveAnswer::SignIn,
            FailureKind::Missing,
            Some(302),
            "로그인",
        ),
        (
            DriveAnswer::Status(403),
            FailureKind::Missing,
            Some(403),
            "공개",
        ),
        (
            DriveAnswer::Status(503),
            FailureKind::Network,
            Some(503),
            "주지 못했어요",
        ),
        (
            DriveAnswer::Redirect {
                host: "collect.example".into(),
            },
            FailureKind::Changed,
            Some(302),
            "따라갈 수 없는",
        ),
    ];
    for (answer, kind, status, says) in cases {
        server.drive("1episode0013", vec![answer.clone()]);
        let failure = source.fetch(&post, file).await.err().unwrap();
        assert_eq!((failure.kind, failure.status), (kind, status), "{answer:?}");
        assert!(
            failure.reason.contains(says),
            "{answer:?}: {}",
            failure.reason
        );
    }
    let failure = {
        server.drive("1episode0013", vec![DriveAnswer::Confirm]);
        source.fetch(&post, file).await.err().unwrap()
    };
    assert_eq!(failure.content_type.as_deref(), Some("text/html"));
    assert!(failure.size.is_some_and(|s| s > 0));
    // No request left Drive's hosts, and none went to sign in.
    assert!(server
        .seen()
        .iter()
        .all(|s| s.host != "collect.example" && s.host != "accounts.google.com"));

    // Within Drive's hosts a redirect is followed.
    server.drive(
        "1episode0013",
        vec![
            DriveAnswer::Redirect {
                host: "drive.google.com".into(),
            },
            DriveAnswer::File {
                name: "13.ass".into(),
                bytes: crate::fake::ass("13"),
            },
        ],
    );
    let (_, bytes) = bytes_of(&source, &post, file).await;
    assert_eq!(bytes, crate::fake::ass("13"));
    // The cookie Drive set on the first answer is not sent on the
    // redirect to its other host, nor on any later request.
    let seen = server.seen();
    let followed = seen
        .iter()
        .find(|s| s.host == "drive.google.com" && s.path == "/download")
        .unwrap();
    assert!(!followed.cookie && !followed.referer);
    assert!(seen.iter().all(|s| !s.cookie && !s.referer));
}

#[tokio::test]
async fn a_redirect_is_followed_on_the_cdn_with_no_referer_and_stopped_elsewhere() {
    let server = SourceServer::start().await;
    let zip = verify::zip_of(&[("a.srt", SRT)]);
    server.post(
        "blog",
        1,
        vec![PostAnswer::Files(vec![
            spec("r1", "a.zip", "1KB"),
            spec("r2", "b.zip", "1KB"),
        ])],
    );
    server.file(
        "r1",
        vec![FileAnswer::Redirect {
            host: "t1.daumcdn.net".into(),
            id: "z1".into(),
        }],
    );
    server.file("z1", vec![FileAnswer::Bytes(zip.clone())]);
    server.file(
        "r2",
        vec![FileAnswer::Redirect {
            host: "collect.example".into(),
            id: "z1".into(),
        }],
    );
    let source = Source::Tistory(server.source());
    let post = Url::parse(&server.post_url("blog", 1)).unwrap();
    let files = files(&source, &post).await;

    // To another CDN host: followed, and the signed address it came from
    // goes along in no `Referer`.
    let (_, bytes) = bytes_of(&source, &post, &files[0]).await;
    assert_eq!(bytes, zip);
    let seen = server.seen();
    let followed = seen.iter().find(|s| s.host == "t1.daumcdn.net").unwrap();
    assert!(!followed.referer && !followed.cookie);

    // To any other host: not followed, and the answer is the redirect's.
    let failure = source.fetch(&post, &files[1]).await.err().unwrap();
    assert_eq!(failure.kind, FailureKind::Changed);
    assert_eq!(failure.status, Some(302));
    assert!(!failure.reason.contains("signature") && !failure.reason.contains("example"));
    assert!(server.seen().iter().all(|s| s.host != "collect.example"));
    assert!(server.seen().iter().all(|s| !s.referer && !s.cookie));
}

#[tokio::test]
async fn a_file_past_its_byte_limit_or_encoded_is_not_a_file() {
    let server = SourceServer::start().await;
    let big = vec![b'x'; 4096];
    server.post(
        "blog",
        1,
        vec![PostAnswer::Files(vec![
            spec("said", "a.srt", "4KB"),
            spec("sent", "b.srt", "4KB"),
            spec("small", "c.srt", "1KB"),
            spec("gz", "d.srt", "1KB"),
        ])],
    );
    server.file("said", vec![FileAnswer::Bytes(big.clone())]);
    server.file("sent", vec![FileAnswer::Streamed(big.clone())]);
    server.file("small", vec![FileAnswer::Streamed(SRT.to_vec())]);
    server.file("gz", vec![FileAnswer::Encoded(SRT.to_vec())]);
    let source = Source::Tistory(server.source_with(tistory::Limits {
        spacing: Duration::ZERO,
        max_file: 2048,
        ..tistory::Limits::default()
    }));
    let post = Url::parse(&server.post_url("blog", 1)).unwrap();
    let files = files(&source, &post).await;

    // Announced past the limit: refused before a byte is read.
    let failure = source.fetch(&post, &files[0]).await.err().unwrap();
    assert_eq!(
        (failure.kind, failure.status, failure.size),
        (FailureKind::NotAFile, Some(200), Some(4096))
    );
    assert!(failure.reason.contains("2048바이트"));

    // Not announced: the bytes stop at the limit.
    let mut fetch = source.fetch(&post, &files[1]).await.unwrap();
    assert_eq!(fetch.expected_size, None);
    let failure = loop {
        match fetch.chunk().await {
            Ok(Some(_)) => continue,
            Ok(None) => panic!("the bytes went past the limit"),
            Err(failure) => break failure,
        }
    };
    assert_eq!(failure.kind, FailureKind::NotAFile);
    assert!(failure.size.is_some_and(|s| s > 2048 && s <= 3072));

    // Within it: whole.
    let (_, bytes) = bytes_of(&source, &post, &files[2]).await;
    assert_eq!(bytes, SRT);

    // Encoded: not the file, whatever its length says.
    let failure = source.fetch(&post, &files[3]).await.err().unwrap();
    assert_eq!(failure.kind, FailureKind::NotAFile);
    assert!(failure.reason.contains("Content-Encoding"));
}

#[tokio::test]
async fn a_file_that_stalls_past_its_deadline_is_a_network_failure() {
    let server = SourceServer::start().await;
    server.post(
        "blog",
        1,
        vec![PostAnswer::Files(vec![spec("s", "a.srt", "4KB")])],
    );
    server.file("s", vec![FileAnswer::Stalled(vec![b'x'; 4096])]);
    let source = Source::Tistory(server.source_with(tistory::Limits {
        spacing: Duration::ZERO,
        file_deadline: Duration::from_millis(300),
        ..tistory::Limits::default()
    }));
    let post = Url::parse(&server.post_url("blog", 1)).unwrap();
    let files = files(&source, &post).await;
    let started = Instant::now();
    let mut fetch = source.fetch(&post, &files[0]).await.unwrap();
    let failure = loop {
        match fetch.chunk().await {
            Ok(Some(_)) => continue,
            Ok(None) => panic!("a stalled answer has no end"),
            Err(failure) => break failure,
        }
    };
    assert_eq!(failure.kind, FailureKind::Network);
    assert!(failure.reason.contains("1초 안에 다 받지 못했어요"));
    assert!(started.elapsed() < Duration::from_secs(5));
}

#[tokio::test]
async fn requests_to_one_host_are_spaced() {
    let server = SourceServer::start().await;
    let specs: Vec<FileSpec> = (0..3)
        .map(|i| spec(&format!("f{i}"), &format!("{i}.srt"), "1KB"))
        .collect();
    server.post("blog", 1, vec![PostAnswer::Files(specs.clone())]);
    for s in &specs {
        server.file(&s.id, vec![FileAnswer::Bytes(SRT.to_vec())]);
    }
    // The server sees each request a little after it is sent, by an
    // amount a loaded machine can stretch: a wide margin under the spacing.
    let source = Source::Tistory(server.source_spaced(Duration::from_millis(300)));
    let post = Url::parse(&server.post_url("blog", 1)).unwrap();
    for file in files(&source, &post).await {
        bytes_of(&source, &post, &file).await;
    }
    let cdn: Vec<Instant> = server
        .seen()
        .iter()
        .filter(|s| s.host == CDN)
        .map(|s| s.at)
        .collect();
    assert_eq!(cdn.len(), 3);
    assert!(cdn
        .windows(2)
        .all(|w| w[1] - w[0] >= Duration::from_millis(200)));
}

#[tokio::test]
async fn sources_sharing_one_drive_space_their_drive_requests_together() {
    let server = SourceServer::start().await;
    let unspaced = Limits {
        spacing: Duration::ZERO,
        ..Limits::default()
    };
    let drive = server.drive_with(Limits {
        spacing: Duration::from_millis(300),
        ..Limits::default()
    });
    let tistory = Source::Tistory(server.source_over(unspaced, drive.clone()));
    let blogger = Source::Blogger(server.blogger_over(unspaced, drive));
    server.post(
        "felia",
        1187,
        vec![PostAnswer::Page(drive_page("1tistoryfile0001"))],
    );
    let tistory_post = Url::parse(&server.post_url("felia", 1187)).unwrap();
    let blogger_post = csora(&server);
    for (id, name) in [("1tistoryfile0001", "01.ass"), ("1episode0013", "13.ass")] {
        server.drive(
            id,
            vec![DriveAnswer::File {
                name: name.into(),
                bytes: crate::fake::ass(name),
            }],
        );
    }
    let Opened::Files(from_tistory) = tistory.open(&tistory_post, "1").await.unwrap() else {
        panic!("files");
    };
    let Opened::Files(from_blogger) = blogger.open(&blogger_post, "13").await.unwrap() else {
        panic!("files");
    };

    for _ in 0..2 {
        bytes_of(&tistory, &tistory_post, &from_tistory[0]).await;
        bytes_of(&blogger, &blogger_post, &from_blogger[1]).await;
    }
    let drive: Vec<Instant> = server
        .seen()
        .iter()
        .filter(|s| crate::drive::HOSTS.contains(&s.host.as_str()))
        .map(|s| s.at)
        .collect();
    assert_eq!(drive.len(), 4);
    assert!(drive
        .windows(2)
        .all(|w| w[1] - w[0] >= Duration::from_millis(200)));
}

#[tokio::test]
async fn drive_and_the_cdn_set_a_cookie_the_sources_never_send() {
    // The cookie checks above mean something only while these hosts
    // set one.
    let server = SourceServer::start().await;
    let client = builder().build().unwrap();
    for (url, domain) in [
        (
            format!("http://{DRIVE_FILES}:{}/download?id=x", server.port),
            "domain=.google.com",
        ),
        (
            format!("http://{CDN}:{}/dna/x/y/z/a.zip", server.port),
            "domain=.kakaocdn.net",
        ),
    ] {
        let response = client.get(&url).send().await.unwrap();
        let cookie = response.headers().get(header::SET_COOKIE).unwrap();
        let cookie = cookie.to_str().unwrap();
        assert!(
            cookie.starts_with("NID=") && cookie.contains(domain),
            "{cookie}"
        );
    }
}

/// 공룡이's post of 네죽사 8화 (2026-10-03): a ZIP of 1~8화 and the ASS
/// of 8화, inside the inner frame.
fn elaina(server: &SourceServer) -> (Url, Vec<u8>, Vec<u8>) {
    let zip = verify::zip_of(&[("네죽사 08.ass", crate::fake::ass("08").as_slice())]);
    let ass = crate::fake::ass("Kimi ga Shinu 08");
    server.naver_post(
        "elainalove1017",
        "224324105274",
        vec![PostAnswer::Naver(vec![
            naver_file("네죽사 1~8화 자막.zip", zip.len()),
            naver_file(ELAINA_ASS, ass.len()),
        ])],
    );
    (
        Url::parse(&server.naver_url("elainalove1017", "224324105274")).unwrap(),
        zip,
        ass,
    )
}

const ELAINA_ASS: &str =
    "[SubsPlease] Kimi ga Shinu made Koi wo Shitai - 08 (1080p) [F3B053C5].ass";

#[tokio::test]
async fn a_naver_posts_attachments_in_its_inner_frame_are_received() {
    let server = SourceServer::start().await;
    let (post, zip, ass) = elaina(&server);
    server.naver_attachment(ELAINA_ASS, vec![FileAnswer::Bytes(ass.clone())]);
    server.naver_attachment(
        "네죽사 1~8화 자막.zip",
        vec![FileAnswer::Bytes(zip.clone())],
    );
    let source = Source::Naver(server.naver());

    let Opened::Files(files) = source.open(&post, "8").await.unwrap() else {
        panic!("files");
    };
    let names: Vec<&str> = files.iter().map(|f| f.name.as_str()).collect();
    assert_eq!(names, ["네죽사 1~8화 자막.zip", ELAINA_ASS]);
    assert_eq!(
        files[1].snapshot.entries(),
        [
            (naver::PUBLISH_DATE.to_owned(), NAVER_PUBLISHED.to_owned()),
            (naver::ATTACH_FILE_SIZE.to_owned(), ass.len().to_string()),
        ]
    );
    for (file, bytes) in files.iter().zip([zip, ass]) {
        let (expected, got) = bytes_of(&source, &post, file).await;
        assert_eq!((expected, got), (Some(bytes.len() as u64), bytes));
    }
    // The inner page was read, never the frame around it; nothing carried
    // a cookie or a `Referer`.
    let seen = server.seen();
    assert_eq!(server.naver_read("224324105274"), 1);
    assert!(seen
        .iter()
        .all(|s| s.path != "/elainalove1017/224324105274"));
    assert_eq!(
        seen.iter().filter(|s| s.host == naver::FILE_HOST).count(),
        2
    );
    assert!(seen.iter().all(|s| !s.cookie && !s.referer));

    // Episode 9: neither file.
    let failure = source.open(&post, "9").await.unwrap_err();
    assert_eq!(failure.kind, FailureKind::Changed);
    assert!(failure.reason.contains("9화"));
}

#[tokio::test]
async fn a_refused_naver_address_is_read_again_once_then_expired_or_missing() {
    let server = SourceServer::start().await;
    let (post, _, ass) = elaina(&server);
    let source = Source::Naver(server.naver());
    let Opened::Files(files) = source.open(&post, "8").await.unwrap() else {
        panic!("files");
    };
    let file = &files[1];

    // Refused once: the post is read again, and the new address works.
    server.naver_attachment(
        ELAINA_ASS,
        vec![FileAnswer::Refused, FileAnswer::Bytes(ass.clone())],
    );
    let (_, bytes) = bytes_of(&source, &post, file).await;
    assert_eq!(bytes, ass);
    assert_eq!(server.naver_read("224324105274"), 2);
    let tokens: Vec<String> = server
        .seen()
        .iter()
        .filter(|s| s.host == naver::FILE_HOST)
        .map(|s| s.path.split('/').nth(3).unwrap().to_owned())
        .collect();
    assert_eq!(tokens.len(), 2);
    assert_ne!(tokens[0], tokens[1], "the second address is the new one");

    // Refused again: expired.
    server.naver_attachment(ELAINA_ASS, vec![FileAnswer::Refused]);
    let failure = source.fetch(&post, file).await.err().unwrap();
    assert_eq!(failure.kind, FailureKind::Expired);
    assert_eq!(failure.status, Some(400));
    assert!(failure.reason.contains("다시 읽어"));
    assert!(!failure.reason.contains("open") && !failure.reason.contains("T1"));

    // Gone from the post read again: missing.
    server.naver_post(
        "elainalove1017",
        "224324105274",
        vec![PostAnswer::Naver(vec![naver_file(
            "네죽사 1~8화 자막.zip",
            10,
        )])],
    );
    let failure = source.fetch(&post, file).await.err().unwrap();
    assert_eq!(failure.kind, FailureKind::Missing);
    assert!(failure.reason.contains("파일이 없어요"));
}

#[tokio::test]
async fn a_flagged_naver_file_is_never_asked_for() {
    let server = SourceServer::start().await;
    let mut flagged = naver_file("Title 08.ass", 100);
    flagged.malicious = true;
    let mut punished = naver_file("Title 08.smi", 100);
    punished.punish = "2".to_owned();
    server.naver_post(
        "blog",
        "1",
        vec![PostAnswer::Naver(vec![flagged, punished])],
    );
    let post = Url::parse(&server.naver_url("blog", "1")).unwrap();
    let source = Source::Naver(server.naver());
    let Opened::Files(files) = source.open(&post, "8").await.unwrap() else {
        panic!("files");
    };
    let failure = source.fetch(&post, &files[0]).await.err().unwrap();
    assert_eq!(failure.kind, FailureKind::Missing);
    assert!(failure.reason.contains("악성 코드"));
    let failure = source.fetch(&post, &files[1]).await.err().unwrap();
    assert_eq!(failure.kind, FailureKind::Missing);
    assert!(failure.reason.contains("제한"));
    assert!(server.seen().iter().all(|s| s.host != naver::FILE_HOST));
    assert_eq!(server.naver_read("1"), 1);
}

#[tokio::test]
async fn a_naver_answer_is_held_to_the_size_the_post_gave() {
    let server = SourceServer::start().await;
    server.naver_post(
        "blog",
        "2",
        vec![PostAnswer::Naver(vec![
            naver_file("Title 03.srt", SRT.len() + 1),
            naver_file("Title 04.srt", SRT.len()),
        ])],
    );
    server.naver_attachment("Title 03.srt", vec![FileAnswer::Bytes(SRT.to_vec())]);
    server.naver_attachment("Title 04.srt", vec![FileAnswer::Streamed(SRT.to_vec())]);
    let post = Url::parse(&server.naver_url("blog", "2")).unwrap();
    let source = Source::Naver(server.naver());
    let open = |episode: &'static str| {
        let source = source.clone();
        let post = post.clone();
        async move {
            match source.open(&post, episode).await.unwrap() {
                Opened::Files(files) => files,
                other => panic!("files: {other:?}"),
            }
        }
    };
    // Another length announced: not the file.
    let file = &open("3").await[0];
    let failure = source.fetch(&post, file).await.err().unwrap();
    assert_eq!(failure.kind, FailureKind::NotAFile);
    assert!(failure.reason.contains(&format!("{}바이트", SRT.len() + 1)));
    // No length announced: the post's is the one the bytes are held to.
    let file = &open("4").await[0];
    let (expected, bytes) = bytes_of(&source, &post, file).await;
    assert_eq!((expected, bytes), (Some(SRT.len() as u64), SRT.to_vec()));
}

#[tokio::test]
async fn a_page_instead_of_the_naver_post_is_a_classified_failure() {
    let server = SourceServer::start().await;
    let source = Source::Naver(server.naver());
    let cases = [
            (
                PostAnswer::Page(
                    r#"<html><body><form><img id="captchaimg" src="x"><p>자동입력 방지 문자를 입력해 주세요</p></form></body></html>"#
                        .to_owned(),
                ),
                FailureKind::Changed,
                "CAPTCHA",
            ),
            (
                PostAnswer::Page("<html><body>점검 중이에요</body></html>".to_owned()),
                FailureKind::Changed,
                "다른 페이지",
            ),
            (PostAnswer::Status(404), FailureKind::Missing, "없어요"),
            (PostAnswer::Status(503), FailureKind::Network, "주지 못했어요"),
        ];
    let post = Url::parse(&server.naver_url("blog", "3")).unwrap();
    for (answer, kind, says) in cases {
        server.naver_post("blog", "3", vec![answer.clone()]);
        let failure = source.open(&post, "1").await.unwrap_err();
        assert_eq!(failure.kind, kind, "{answer:?}");
        assert!(
            failure.reason.contains(says),
            "{answer:?}: {}",
            failure.reason
        );
    }
    // An address of no post is asked for nothing.
    let home = Url::parse(&format!("http://blog.naver.com:{}/blog", server.port)).unwrap();
    let before = server.seen().len();
    let failure = source.open(&home, "1").await.unwrap_err();
    assert_eq!(failure.kind, FailureKind::Changed);
    assert_eq!(server.seen().len(), before);
    assert!(server.seen().iter().all(|s| s.host != naver::FILE_HOST));
}

fn keys(names: &[&str]) -> Vec<String> {
    names.iter().map(|n| (*n).to_owned()).collect()
}

#[tokio::test]
async fn a_drive_file_is_read_again_by_a_head_with_no_body_cookie_or_referer() {
    let server = SourceServer::start().await;
    let source = Source::Blogger(server.blogger());
    let post = csora(&server);
    let ass = crate::fake::ass("Seihantai 24");
    server.drive(
        "1episode0024",
        vec![DriveAnswer::FileAt {
            name: "Seihantai 24.ass".into(),
            bytes: ass.clone(),
            modified: "Tue, 29 Sep 2026 19:53:19 GMT".into(),
        }],
    );
    let answers = source.recheck(&post, &keys(&["drive:1episode0024"])).await;
    assert_eq!(answers.len(), 1);
    assert_eq!(
        answers[0].1.as_ref().unwrap(),
        &FileInfo {
            size: Some(ass.len() as u64),
            last_modified: Some("Tue, 29 Sep 2026 19:53:19 GMT".into()),
        }
    );
    // One `HEAD` of the download address; the post is not read and nothing
    // else is asked for.
    let seen = server.seen();
    assert_eq!(seen.len(), 1);
    assert_eq!(
        (seen[0].method.as_str(), seen[0].host.as_str()),
        ("HEAD", DRIVE_FILES)
    );
    assert_eq!(seen[0].path, "/download");
    assert!(!seen[0].cookie && !seen[0].referer && seen[0].range.is_none());

    // A file gone, one that asks to sign in, a page instead of the file.
    for (answer, kind) in [
        (DriveAnswer::Missing, FailureKind::Missing),
        (DriveAnswer::SignIn, FailureKind::Missing),
        (DriveAnswer::Status(503), FailureKind::Network),
        (DriveAnswer::Confirm, FailureKind::NotAFile),
    ] {
        server.drive("1episode0024", vec![answer.clone()]);
        let answers = source.recheck(&post, &keys(&["drive:1episode0024"])).await;
        assert_eq!(answers[0].1.as_ref().unwrap_err().kind, kind, "{answer:?}");
    }
    // A key that is no Drive file's is no Blogger file.
    let answers = source.recheck(&post, &keys(&["tistory:x"])).await;
    assert_eq!(
        answers[0].1.as_ref().unwrap_err().kind,
        FailureKind::Changed
    );
}

#[tokio::test]
async fn a_tistory_attachment_is_read_again_by_a_one_byte_range_for_its_total() {
    let server = SourceServer::start().await;
    let source = Source::Tistory(server.source());
    let post = Url::parse(&server.post_url("sumomomo", 491)).unwrap();
    let (a, b) = (spec("a", "a.zip", "0.01MB"), spec("b", "b.zip", "0.02MB"));
    server.post("sumomomo", 491, vec![PostAnswer::Files(vec![a, b])]);
    server.file("a", vec![FileAnswer::Bytes(vec![7; 11_724])]);
    server.file("b", vec![FileAnswer::Bytes(vec![8; 90])]);
    let offered = files(&source, &post).await;
    let wanted: Vec<String> = offered.iter().map(|f| f.key.clone()).collect();

    let answers = source.recheck(&post, &wanted).await;
    let sizes: Vec<Option<u64>> = answers
        .iter()
        .map(|(_, info)| info.as_ref().unwrap().size)
        .collect();
    assert_eq!(sizes, [Some(11_724), Some(90)]);
    // No `Last-Modified` or `ETag` on that CDN: a size is all there is.
    assert!(answers
        .iter()
        .all(|(_, info)| info.as_ref().unwrap().last_modified.is_none()));
    // The post once for both files (and once before, by `open`); each
    // address asked for one byte, never with a `HEAD`, no cookie, no
    // `Referer`, and no body of the file taken.
    let seen = server.seen();
    let posts = seen
        .iter()
        .filter(|s| s.host.ends_with(".tistory.com"))
        .count();
    assert_eq!(posts, 2);
    let cdn: Vec<_> = seen.iter().filter(|s| s.host == CDN).collect();
    assert_eq!(cdn.len(), 2);
    assert!(cdn
        .iter()
        .all(|s| s.method == "GET" && s.range.as_deref() == Some("bytes=0-0")));
    assert!(seen.iter().all(|s| !s.cookie && !s.referer));

    // A file the post no longer offers, and an address the CDN refuses.
    server.post(
        "sumomomo",
        491,
        vec![PostAnswer::Files(vec![spec("a", "a.zip", "0.01MB")])],
    );
    server.file("a", vec![FileAnswer::Refused]);
    let answers = source.recheck(&post, &wanted).await;
    assert_eq!(
        answers[0].1.as_ref().unwrap_err().kind,
        FailureKind::Expired
    );
    assert_eq!(
        answers[1].1.as_ref().unwrap_err().kind,
        FailureKind::Missing
    );

    // The post gone: every file of it, and a CDN that is down.
    server.post("sumomomo", 491, vec![PostAnswer::Status(404)]);
    let answers = source.recheck(&post, &wanted).await;
    assert!(answers
        .iter()
        .all(|(_, info)| info.as_ref().unwrap_err().kind == FailureKind::Missing));
    server.post(
        "sumomomo",
        491,
        vec![PostAnswer::Files(vec![spec("a", "a.zip", "1KB")])],
    );
    server.file("a", vec![FileAnswer::Status(503)]);
    let answers = source.recheck(&post, &wanted[..1]).await;
    assert_eq!(
        answers[0].1.as_ref().unwrap_err().kind,
        FailureKind::Network
    );
}

#[tokio::test]
async fn a_naver_attachment_is_read_again_from_the_posts_page_alone() {
    let server = SourceServer::start().await;
    let (post, zip, ass) = elaina(&server);
    let source = Source::Naver(server.naver());
    let wanted = keys(&[
        "naver:elainalove1017/224324105274/네죽사 1~8화 자막.zip",
        &format!("naver:elainalove1017/224324105274/{ELAINA_ASS}"),
        "naver:elainalove1017/224324105274/gone.ass",
    ]);
    let answers = source.recheck(&post, &wanted).await;
    assert_eq!(
        answers[0].1.as_ref().unwrap(),
        &FileInfo {
            size: Some(zip.len() as u64),
            last_modified: None
        }
    );
    assert_eq!(answers[1].1.as_ref().unwrap().size, Some(ass.len() as u64));
    assert_eq!(
        answers[2].1.as_ref().unwrap_err().kind,
        FailureKind::Missing
    );
    // One reading of the inner page, none of the file host.
    assert_eq!(server.naver_read("224324105274"), 1);
    assert!(server.seen().iter().all(|s| s.host != naver::FILE_HOST));

    // A flagged file and a post gone.
    server.naver_post(
        "elainalove1017",
        "224324105274",
        vec![PostAnswer::Naver(vec![NaverFile {
            malicious: true,
            ..naver_file("a.zip", 10)
        }])],
    );
    let answers = source
        .recheck(&post, &keys(&["naver:elainalove1017/224324105274/a.zip"]))
        .await;
    assert_eq!(
        answers[0].1.as_ref().unwrap_err().kind,
        FailureKind::Missing
    );
    server.naver_post(
        "elainalove1017",
        "224324105274",
        vec![PostAnswer::Status(404)],
    );
    let answers = source.recheck(&post, &wanted[..1]).await;
    assert_eq!(
        answers[0].1.as_ref().unwrap_err().kind,
        FailureKind::Missing
    );
}

#[tokio::test]
async fn an_erulabo_post_is_read_again_for_its_modified_time_and_its_drive_file_by_a_head() {
    use crate::Received;

    let server = SourceServer::start().await;
    let source = Source::Erulabo(server.erulabo());
    let post = Url::parse(&server.erulabo_url(859)).unwrap();
    server.erulabo_post(
        859,
        vec![PostAnswer::Page(erulabo_page(
            &[("/file/aaaa-1", "전생귀족3 (1)")],
            "2026-10-02T09:00:00+09:00",
        ))],
    );
    let bytes = crate::fake::ass("erulabo 1");
    server.drive(
        "1erulabodrive01",
        vec![DriveAnswer::FileAt {
            name: "a.zip".into(),
            bytes: bytes.clone(),
            modified: "Tue, 29 Sep 2026 19:53:19 GMT".into(),
        }],
    );
    let received = vec![
        Received::new("browser:erulabo.com/859#a.zip", Some("1erulabodrive01")),
        // The same file twice (a series card for two episodes) is asked
        // for once; a file with no Drive ID, or an odd one, is not.
        Received::new("browser:erulabo.com/859#a.zip", Some("1erulabodrive01")),
        Received::new("browser:erulabo.com/859#b.zip", None),
        Received::new("browser:erulabo.com/859#c.zip", Some("a b")),
        // Another name for the same Drive file shares the one answer.
        Received::new("browser:erulabo.com/859#d.zip", Some("1erulabodrive01")),
    ];
    let reading = source
        .recheck_post(&post, &received)
        .await
        .expect("erulabo reads the post")
        .unwrap();
    assert_eq!(
        reading.modified.as_deref(),
        Some("2026-10-02T09:00:00+09:00")
    );
    assert_eq!(reading.observed.len(), 2);
    assert_eq!(reading.observed[0].0, "browser:erulabo.com/859#a.zip");
    assert_eq!(reading.observed[1].0, "browser:erulabo.com/859#d.zip");
    assert_eq!(
        reading.observed[0].1.as_ref().unwrap(),
        &FileInfo {
            size: Some(bytes.len() as u64),
            last_modified: Some("Tue, 29 Sep 2026 19:53:19 GMT".into()),
        }
    );
    assert_eq!(reading.observed[1].1, reading.observed[0].1);
    // The post once (a `GET`, no cookie), the Drive file once (a `HEAD`,
    // no cookie, no `Referer`); the check's card was never clicked.
    let seen = server.seen();
    assert_eq!(seen.len(), 2);
    assert_eq!(
        (
            seen[0].method.as_str(),
            seen[0].host.as_str(),
            seen[0].path.as_str()
        ),
        ("GET", "erulabo.com", "/859")
    );
    assert_eq!(
        (seen[1].method.as_str(), seen[1].host.as_str()),
        ("HEAD", DRIVE_FILES)
    );
    assert!(seen.iter().all(|s| !s.cookie && !s.referer));
    // Neither the file's ID nor an address is shown by the types.
    assert!(!format!("{received:?}").contains("1erulabodrive01"));

    // A Drive file that cannot be read is its own failure; the post is
    // still read.
    server.drive("1erulabodrive01", vec![DriveAnswer::Missing]);
    let reading = source
        .recheck_post(&post, &received)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        reading.modified.as_deref(),
        Some("2026-10-02T09:00:00+09:00")
    );
    let failure = reading.observed[0].1.as_ref().unwrap_err();
    assert_eq!(failure.kind, FailureKind::Missing);
    assert!(!failure.reason.contains("1erulabodrive01"));

    // A post gone is the failure, and Drive is not asked then.
    server.erulabo_post(859, vec![PostAnswer::Status(404)]);
    let before = server.seen().len();
    let failure = source
        .recheck_post(&post, &received)
        .await
        .unwrap()
        .unwrap_err();
    assert_eq!(failure.kind, FailureKind::Missing);
    let after: Vec<_> = server.seen().split_off(before);
    assert_eq!(after.len(), 1);
    assert_eq!(after[0].host, "erulabo.com");

    // A page that is not the post (no body) is a post that changed; a
    // post that does not say when it was modified has no time.
    server.erulabo_post(
        859,
        vec![PostAnswer::Page("<html><body>점검 중</body></html>".into())],
    );
    let failure = source
        .recheck_post(&post, &received)
        .await
        .unwrap()
        .unwrap_err();
    assert_eq!(failure.kind, FailureKind::Changed);
    server.erulabo_post(
        859,
        vec![PostAnswer::Page(
            r#"<html><body><div id="post-body"></div></body></html>"#.into(),
        )],
    );
    let reading = source
        .recheck_post(&post, &received)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(reading.modified, None);

    // The other sources read their files directly.
    let tistory = Source::Tistory(server.source());
    assert!(tistory.recheck_post(&post, &received).await.is_none());
}
