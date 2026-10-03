//! The WinPNG reader against a real viewer, in the real browser image
//! (ignored). It opens real posts of a Tistory blog whose skin carries the
//! viewer, takes the files out of their images and prints what came out; it
//! also makes the viewer see a picture and an image it cannot read, to show
//! what the reader does with each.
//!
//! Needs Docker and the browser image (`TRSS_BROWSER_IMAGE`, default
//! `ghcr.io/syrflover/trss-browser:local`, built from `Dockerfile.browser`)
//! and the posts to read (`TRSS_WINPNG_POSTS`, addresses separated by spaces):
//!
//! ```sh
//! TRSS_WINPNG_POSTS="https://harne1.tistory.com/762 https://harne1.tistory.com/763" \
//!     cargo test -p trss-subtitles --features test-hooks --test winpng_sample -- --ignored --nocapture
//! ```
//!
//! The test needs the `test-hooks` feature (it is not built without it). It
//! starts a throwaway container per test, named
//! `trss-winpng-sample-<test>-<pid>` (removed at the end). Only counts,
//! sizes and formats are printed: no address.

use std::{
    path::Path,
    process::{Command, Output},
    time::Duration,
};

use trss_browser::{BrowserPolicy, BrowserPool, PolicySource, PoolConfig};
use trss_subtitles::{
    verify,
    winpng::{BrowserReader, ViewRequest, Viewed, WinpngReader},
};
use url::Url;

const TOKEN: &str = "sample-token";

fn image() -> String {
    std::env::var("TRSS_BROWSER_IMAGE")
        .unwrap_or_else(|_| "ghcr.io/syrflover/trss-browser:local".to_owned())
}

fn docker(args: &[&str]) -> Output {
    Command::new("docker").args(args).output().expect("docker")
}

fn docker_ok(args: &[&str]) -> String {
    let out = docker(args);
    assert!(
        out.status.success(),
        "docker {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim().to_owned()
}

/// Removes the container at the end, whatever happened.
struct Container(String);

impl Drop for Container {
    fn drop(&mut self) {
        let _ = docker(&["rm", "-f", &self.0]);
    }
}

async fn start(test: &str, downloads: &Path) -> (Container, BrowserPool) {
    // One name per test: the tests run in parallel by default.
    let name = format!("trss-winpng-sample-{test}-{}", std::process::id());
    let container = Container(name.clone());
    std::fs::set_permissions(
        downloads,
        std::os::unix::fs::PermissionsExt::from_mode(0o1777),
    )
    .unwrap();
    docker_ok(&[
        "run",
        "-d",
        "--name",
        &name,
        "--memory",
        "768m",
        "--cap-drop",
        "ALL",
        "--security-opt",
        "no-new-privileges:true",
        "-e",
        &format!("TRSS_BROWSER_TOKEN={TOKEN}"),
        "-v",
        &format!("{}:/downloads", downloads.display()),
        &image(),
    ]);
    let ip = docker_ok(&[
        "inspect",
        "-f",
        "{{range .NetworkSettings.Networks}}{{.IPAddress}}{{end}}",
        &name,
    ]);
    let base = format!("http://{ip}:9230");
    let client = reqwest::Client::new();
    for _ in 0..80 {
        let up = client
            .get(format!("{base}/runs"))
            .bearer_auth(TOKEN)
            .send()
            .await
            .map(|r| r.status().is_success())
            .unwrap_or(false);
        if up {
            break;
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    let pool = BrowserPool::new(
        PoolConfig::new(base.parse().unwrap(), TOKEN, downloads),
        trss_core::system_clock(),
        PolicySource::fixed(BrowserPolicy::default()),
    )
    .await
    .unwrap();
    (container, pool)
}

/// Makes every image the page fetches from the CDN a plain picture, as a post
/// whose image holds nothing would show.
const PLAIN: &str = r#"(async () => {
  const c = document.createElement('canvas'); c.width = 640; c.height = 360;
  const x = c.getContext('2d');
  const g = x.createLinearGradient(0, 0, 640, 360); g.addColorStop(0, '#3a6'); g.addColorStop(1, '#c42');
  x.fillStyle = g; x.fillRect(0, 0, 640, 360);
  for (let i = 0; i < 300; i++) { x.fillStyle = `hsl(${i * 7 % 360},60%,50%)`; x.fillRect((i * 37) % 620, (i * 53) % 340, 20, 20); }
  const blob = await new Promise(r => c.toBlob(r, 'image/png'));
  const real = window.fetch;
  window.fetch = async (u, o) => (typeof u === 'string' && u.includes('kakaocdn')) ? new Response(blob, {status: 200, headers: {'content-type': 'image/png'}}) : real(u, o);
})()"#;

/// Makes the viewer fail to read the image, as one that needs a key does: the
/// viewer asks for the key in a dialog.
const UNREADABLE: &str = "WithTarget.fromBitmap = async () => null;";

fn posts() -> Vec<Url> {
    std::env::var("TRSS_WINPNG_POSTS")
        .expect("TRSS_WINPNG_POSTS names the posts to read")
        .split_whitespace()
        .map(|p| Url::parse(p).unwrap())
        .collect()
}

#[tokio::test]
#[ignore]
async fn the_real_viewer_gives_the_files_of_real_posts() {
    let downloads = tempfile::tempdir().unwrap();
    let (_container, pool) = start("posts", downloads.path()).await;
    let reader = BrowserReader::new(pool.clone());
    let posts = posts();

    for (n, post) in posts.iter().enumerate() {
        let staging = tempfile::tempdir().unwrap();
        let job = format!("sample-{n}");
        let started = std::time::Instant::now();
        let viewed = reader
            .read(ViewRequest {
                job: &job,
                post,
                staging: staging.path(),
            })
            .await
            .unwrap_or_else(|failure| panic!("post {n}: {failure}"));
        let Viewed::Files(files) = viewed else {
            panic!("post {n}: {viewed:?}");
        };

        // Counts, sizes and formats of what came out.
        let mut total = 0u64;
        let mut by_format: std::collections::BTreeMap<String, usize> = Default::default();
        let mut folders: std::collections::BTreeSet<String> = Default::default();
        let mut starts_sami = 0usize;
        for file in &files {
            let bytes = std::fs::read(&file.path).unwrap();
            assert!(!bytes.is_empty(), "post {n}: {} is empty", file.name);
            total += bytes.len() as u64;
            let format = verify::check(&file.path, &file.name)
                .map(|f| f.label().to_owned())
                .unwrap_or_else(|f| panic!("post {n}: {} is no file: {}", file.name, f.reason));
            *by_format.entry(format).or_default() += 1;
            folders.insert(file.folder.clone().unwrap_or_default());
            let text = bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(&bytes);
            if text.starts_with(b"<SAMI>") {
                starts_sami += 1;
            }
        }
        println!(
            "post {n}: {} files, {total} bytes in all, formats {by_format:?}, {} start with <SAMI>, \
             folders {:?}, {} image(s), {:.1}s",
            files.len(),
            starts_sami,
            folders,
            files
                .iter()
                .map(|f| f.image.as_str())
                .collect::<std::collections::BTreeSet<_>>()
                .len(),
            started.elapsed().as_secs_f64()
        );
        let mut sizes: Vec<(String, u64)> = files
            .iter()
            .map(|f| {
                (
                    format!(
                        "{}{}",
                        f.folder
                            .as_deref()
                            .map(|d| format!("{d}/"))
                            .unwrap_or_default(),
                        f.name
                    ),
                    std::fs::metadata(&f.path).unwrap().len(),
                )
            })
            .collect();
        sizes.sort();
        for (name, size) in sizes.iter().take(60) {
            println!("  {size:>9}  {name}");
        }
        assert!(!files.is_empty());
        reader.release(&job).await;
    }
}

#[tokio::test]
#[ignore]
async fn a_picture_has_no_subtitle_and_an_unreadable_image_needs_input() {
    let downloads = tempfile::tempdir().unwrap();
    let (_container, pool) = start("failures", downloads.path()).await;
    let post = posts().remove(0);

    let plain = BrowserReader::new(pool.clone()).with_script_after_ready(PLAIN);
    let staging = tempfile::tempdir().unwrap();
    let viewed = plain
        .read(ViewRequest {
            job: "sample-plain",
            post: &post,
            staging: staging.path(),
        })
        .await
        .unwrap();
    println!("a picture: {viewed:?}");
    assert_eq!(viewed, Viewed::NoSubtitle);
    plain.release("sample-plain").await;

    let keyed = BrowserReader::new(pool.clone()).with_script_after_ready(UNREADABLE);
    let staging = tempfile::tempdir().unwrap();
    let viewed = keyed
        .read(ViewRequest {
            job: "sample-keyed",
            post: &post,
            staging: staging.path(),
        })
        .await
        .unwrap();
    println!("an unreadable image: {viewed:?}");
    assert_eq!(viewed, Viewed::NeedsKey);
    keyed.release("sample-keyed").await;
}
