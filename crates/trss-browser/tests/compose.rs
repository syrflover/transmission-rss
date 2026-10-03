//! What the compose definition promises about the browser container: no
//! published port, no Docker socket, the memory limit, the shared folder.

use std::path::PathBuf;

use yaml_serde::Value;

fn repo_file(name: &str) -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(name);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

/// The file with its `<<` merges applied, as Compose reads it.
fn compose() -> Value {
    let mut compose: Value = yaml_serde::from_str(&repo_file("docker-compose.trss.yml")).unwrap();
    compose.apply_merge().unwrap();
    compose
}

fn service<'a>(compose: &'a Value, name: &str) -> &'a Value {
    &compose["services"][name]
}

fn environment<'a>(service: &'a Value, key: &str) -> Option<&'a str> {
    service["environment"][key].as_str()
}

#[test]
fn the_browser_container_publishes_no_port() {
    let compose = compose();
    let browser = service(&compose, "trss-browser");
    assert!(browser.is_mapping(), "no trss-browser service");
    assert!(
        browser.get("ports").is_none(),
        "trss-browser publishes a port"
    );
    assert!(browser.get("expose").is_none());
    // Not on the host's network either, where every port would be the host's.
    assert!(browser.get("network_mode").is_none());
    // And no ports anywhere in the browser's own definition, for any form of
    // the key.
    let text = yaml_serde::to_string(browser).unwrap();
    assert!(!text.contains("ports"), "{text}");
    assert!(!text.contains("9230:") && !text.contains("9222"), "{text}");
}

fn networks_of(service: &Value) -> Vec<&str> {
    service["networks"]
        .as_sequence()
        .unwrap()
        .iter()
        .map(|n| n.as_str().unwrap())
        .collect()
}

#[test]
fn the_browser_shares_a_network_with_the_worker_alone_and_is_not_on_trss_net() {
    let compose = compose();
    // The pages the browser opens could try requests to trss-web (no sign-in)
    // and Transmission, which are on `trss_net`.
    assert_eq!(
        networks_of(service(&compose, "trss-browser")),
        ["browser_net"]
    );
    // The worker reaches it there, and still reaches Transmission.
    let worker = networks_of(service(&compose, "trss-worker"));
    assert!(worker.contains(&"browser_net") && worker.contains(&"trss_net"));
    assert_eq!(
        environment(service(&compose, "trss-worker"), "TRSS_BROWSER_URL"),
        Some("http://trss-browser:9230")
    );
    // Nothing else is on it.
    let on_browser_net: Vec<&str> = compose["services"]
        .as_mapping()
        .unwrap()
        .iter()
        .filter(|(_, s)| networks_of(s).contains(&"browser_net"))
        .map(|(name, _)| name.as_str().unwrap())
        .collect();
    assert_eq!(on_browser_net, ["trss-worker", "trss-browser"]);
    // The web and the network of Transmission stay as they were.
    assert_eq!(networks_of(service(&compose, "trss-web")), ["trss_net"]);
    // The network is the project's own, and open to the internet, which the
    // pages need: not `internal`.
    let net = &compose["networks"]["browser_net"];
    assert!(net.get("external").is_none());
    assert!(net
        .get("internal")
        .is_none_or(|v| v.as_bool() != Some(true)));
    assert_eq!(
        compose["networks"]["trss_net"]["external"].as_bool(),
        Some(true)
    );
}

#[test]
fn nothing_gives_a_container_the_docker_socket() {
    let text = repo_file("docker-compose.trss.yml");
    assert!(!text.contains("docker.sock"));
    assert!(!text.contains("privileged"));
}

#[test]
fn the_browser_container_has_the_memory_limit_and_no_cpu_limit() {
    let compose = compose();
    let limits = &service(&compose, "trss-browser")["deploy"]["resources"]["limits"];
    assert_eq!(limits["memory"].as_str(), Some("768M"));
    assert!(limits.get("cpus").is_none(), "a CPU limit was not decided");
}

#[test]
fn the_browser_container_shares_only_the_downloads_folder_with_the_worker() {
    let compose = compose();
    let browser = service(&compose, "trss-browser");
    let volumes: Vec<&str> = browser["volumes"]
        .as_sequence()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    assert_eq!(
        volumes,
        ["${TRSS_DATA_DIR:-./data}/browser-downloads:/downloads"]
    );

    // The worker sees that folder inside the data folder it mounts.
    let worker = service(&compose, "trss-worker");
    let data_mounted = worker["volumes"]
        .as_sequence()
        .unwrap()
        .iter()
        .any(|v| v.as_str() == Some("${TRSS_DATA_DIR:-./data}:/data"));
    assert!(data_mounted);
    assert_eq!(
        environment(worker, "TRSS_BROWSER_DOWNLOADS"),
        Some("/data/browser-downloads")
    );
}

#[test]
fn the_token_is_required_and_only_the_worker_and_the_browser_get_it() {
    let compose = compose();
    for name in ["trss-browser", "trss-worker"] {
        let token = environment(service(&compose, name), "TRSS_BROWSER_TOKEN").unwrap();
        assert!(
            token.starts_with("${TRSS_BROWSER_TOKEN:?"),
            "{name}: {token}"
        );
    }
    let web = service(&compose, "trss-web");
    assert!(environment(web, "TRSS_BROWSER_TOKEN").is_none());
    assert!(environment(web, "TRSS_BROWSER_URL").is_none());

    let worker = service(&compose, "trss-worker");
    assert_eq!(
        environment(worker, "TRSS_BROWSER_URL"),
        Some("http://trss-browser:9230")
    );
}

#[test]
fn the_dev_environment_has_a_token_and_builds_the_browser_image() {
    let env = repo_file("dev/dev.env");
    assert!(env.lines().any(|l| l.starts_with("TRSS_BROWSER_TOKEN=")));
    let script = repo_file("dev/compose.sh");
    assert!(script.contains("-f \"$REPO/Dockerfile.browser\""));
    assert!(script.contains("ghcr.io/syrflover/trss-browser:local"));
}

#[test]
fn the_release_workflow_builds_the_browser_image_with_the_same_tags() {
    let workflow = repo_file(".github/workflows/deploy.yml");
    assert!(workflow.contains("file: Dockerfile.browser"));
    assert!(workflow.contains("ghcr.io/syrflover/trss-browser:${{ github.ref_name }}"));
    assert!(workflow.contains("ghcr.io/syrflover/trss-browser:latest"));
}

#[test]
fn the_image_runs_the_launcher_as_a_user_without_exposing_a_port() {
    let dockerfile = repo_file("Dockerfile.browser");
    assert!(dockerfile.lines().any(|l| l.trim() == "USER browser"));
    assert!(!dockerfile
        .lines()
        .any(|l| l.trim_start().starts_with("EXPOSE")));
    assert!(dockerfile.contains("trss-browserd"));
}
