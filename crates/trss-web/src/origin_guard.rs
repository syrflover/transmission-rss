//! Who may talk to the web: the `Host` and `Origin` checks in front of every
//! route (the app, `/api` and WebSockets alike).
//!
//! The web has no sign-in; the network in front of it is the boundary
//! (`docs/specs/web-app.md`, 접근 경계와 기기). Two kinds of page could still
//! use it from a browser inside that boundary, and these checks refuse them:
//!
//! - **DNS rebinding.** A page of an attacker's name that later resolves to
//!   this server's LAN address is, to the browser, the same origin as the
//!   server, so it could read and change everything. Its requests carry the
//!   attacker's name as `Host`. So a request is served only when its `Host`
//!   is an IP address, `localhost`, or a name listed in `TRSS_WEB_HOSTS`
//!   ([`AllowedHosts`]); otherwise it is answered `421`.
//! - **Cross-site requests.** A page of another origin cannot read the
//!   answers, but it can send a request that changes something, and it can
//!   open a WebSocket. So a request that is not `GET`, `HEAD` or `OPTIONS`,
//!   and every WebSocket upgrade, is refused (`403`) when its `Origin` is
//!   `null` or is not the request's own origin, or when `Sec-Fetch-Site` says
//!   `cross-site`. A WebSocket upgrade must carry an `Origin`; another
//!   request without one (not from a browser page) goes through.
//!
//! The request's own origin is its `Host`. The scheme is not compared: the
//! web itself serves plain HTTP, and behind a reverse proxy that ends HTTPS
//! the page's origin is `https://` while the connection here is not, and a
//! forwarded-scheme header is not something this server can trust. A port
//! the `Host` leaves out is the default of the `Origin`'s scheme, because the
//! browser leaves the port out of both for the scheme it used, and that
//! scheme is not known here. So when the web is reached without a port (port
//! 80, or 443 behind a proxy), `https://name` and `http://name` both count as
//! its origin: a page on the other default port of the same host is taken
//! for the web's own. Only a server on that same host could serve such a
//! page, and it is not an attacker's page from elsewhere. A reverse proxy
//! must pass the browser's `Host` on (nginx: `proxy_set_header Host $host`),
//! and its name must be in `TRSS_WEB_HOSTS`.
//!
//! A request with no `Host` at all is served: browsers always send one, and
//! a rebinding page always carries its own name, so only clients that are
//! not browser pages (and the in-process tests) come without it.

use std::{
    net::IpAddr,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex,
    },
    time::{Duration, Instant},
};

use axum::{
    extract::{Request, State},
    http::{header, uri::Authority, HeaderMap, HeaderValue, Method, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
    Json,
};
use serde_json::json;

/// The environment variable that lists the host names the web answers to.
pub const HOSTS_VAR: &str = "TRSS_WEB_HOSTS";

/// How often, at most, refusals are noted in the log.
const NOTE_EVERY: Duration = Duration::from_secs(60);
/// The most of a refused `Host` the log shows.
const NOTED_HOST_CHARS: usize = 64;

/// The host names the web answers to besides IP addresses and `localhost`:
/// lower case, without a port or a final dot.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AllowedHosts {
    names: Vec<String>,
}

impl AllowedHosts {
    /// Reads `TRSS_WEB_HOSTS`: names separated by commas, in any case, each
    /// with or without a port (which is not looked at). `None` for an entry
    /// that is not a host name.
    pub fn parse(value: &str) -> Option<AllowedHosts> {
        let mut names = Vec::new();
        for entry in value.split(',').map(str::trim).filter(|e| !e.is_empty()) {
            let authority: Authority = entry.parse().ok()?;
            if entry.contains('@') || authority.as_str() != entry {
                return None;
            }
            names.push(normal_name(authority.host()));
        }
        Some(AllowedHosts { names })
    }

    pub fn names(&self) -> &[String] {
        &self.names
    }

    /// Whether a request whose `Host` is `host` is served.
    pub fn allows(&self, host: &str) -> bool {
        let Some((name, _)) = split_host(host) else {
            return false;
        };
        is_ip(&name) || name == "localhost" || self.names.contains(&name)
    }
}

/// `host[:port]` of a `Host` header or an origin, split, with the name in
/// lower case and without a final dot (an IPv6 address keeps its brackets).
fn split_host(host: &str) -> Option<(String, Option<u16>)> {
    if host.contains('@') || host.contains('/') {
        return None;
    }
    let authority: Authority = host.parse().ok()?;
    if authority.as_str() != host {
        return None;
    }
    Some((normal_name(authority.host()), authority.port_u16()))
}

fn normal_name(host: &str) -> String {
    let lower = host.to_ascii_lowercase();
    match lower.strip_suffix('.') {
        Some(stripped) if !stripped.is_empty() => stripped.to_owned(),
        _ => lower,
    }
}

fn is_ip(name: &str) -> bool {
    name.strip_prefix('[')
        .and_then(|n| n.strip_suffix(']'))
        .unwrap_or(name)
        .parse::<IpAddr>()
        .is_ok()
}

/// The checks with the hosts they allow, and their notes in the log.
pub struct OriginGuard {
    hosts: AllowedHosts,
    refused_hosts: AtomicU64,
    refused_origins: AtomicU64,
    last_note: Mutex<Option<Instant>>,
}

impl OriginGuard {
    pub fn new(hosts: AllowedHosts) -> OriginGuard {
        OriginGuard {
            hosts,
            refused_hosts: AtomicU64::new(0),
            refused_origins: AtomicU64::new(0),
            last_note: Mutex::new(None),
        }
    }

    /// Notes the refusals in the log at most once a minute: counts, and a
    /// shortened, cleaned copy of the last refused `Host` so that an
    /// operator sees which name to list.
    fn note(&self, host: Option<&str>) {
        let mut last = self.last_note.lock().expect("note lock");
        if last.is_some_and(|at| at.elapsed() < NOTE_EVERY) {
            return;
        }
        *last = Some(Instant::now());
        let hosts = self.refused_hosts.load(Ordering::Relaxed);
        let origins = self.refused_origins.load(Ordering::Relaxed);
        match host {
            Some(host) => {
                let shown: String = host
                    .chars()
                    .filter(|c| c.is_ascii_alphanumeric() || ".-:[]".contains(*c))
                    .take(NOTED_HOST_CHARS)
                    .collect();
                eprintln!(
                    "trss-web: refused {hosts} requests for a host name that is not allowed (latest {shown:?}) and {origins} from another origin so far; if the name is this server's, add it to {HOSTS_VAR}"
                );
            }
            None => eprintln!(
                "trss-web: refused {hosts} requests for a host name that is not allowed and {origins} from another origin so far"
            ),
        }
    }

    /// What is wrong with a request, if anything.
    fn check(&self, method: &Method, headers: &HeaderMap, uri_host: Option<&str>) -> Verdict {
        let host = match headers.get(header::HOST) {
            Some(value) => match value.to_str() {
                Ok(host) => Some(host),
                Err(_) => return Verdict::Host(None),
            },
            None => uri_host,
        };
        if let Some(host) = host {
            if !self.hosts.allows(host) {
                return Verdict::Host(Some(host.to_owned()));
            }
        }

        let websocket = method == Method::CONNECT
            || headers
                .get_all(header::UPGRADE)
                .iter()
                .filter_map(|v| v.to_str().ok())
                .flat_map(|v| v.split(','))
                .any(|token| token.trim().eq_ignore_ascii_case("websocket"));
        let changes = !matches!(*method, Method::GET | Method::HEAD | Method::OPTIONS);
        if !websocket && !changes {
            return Verdict::Pass;
        }
        if headers
            .get("sec-fetch-site")
            .is_some_and(|site| site.as_bytes().eq_ignore_ascii_case(b"cross-site"))
        {
            return Verdict::Origin;
        }
        match headers.get(header::ORIGIN) {
            None if websocket => Verdict::Origin,
            None => Verdict::Pass,
            Some(origin) => match (origin.to_str(), host) {
                (Ok(origin), Some(host)) if same_origin(origin, host) => Verdict::Pass,
                _ => Verdict::Origin,
            },
        }
    }
}

enum Verdict {
    Pass,
    /// The `Host` is not allowed (`None`: it is not even text).
    Host(Option<String>),
    Origin,
}

/// Whether `origin` (`scheme://host[:port]`) is the origin of a request
/// whose `Host` is `host`. `null`, and anything that is not an `http` or
/// `https` origin, is not. A `Host` without a port takes the default port of
/// the origin's scheme (see the module's note on why the scheme is not
/// compared).
fn same_origin(origin: &str, host: &str) -> bool {
    let (default_port, rest) = if let Some(rest) = origin.strip_prefix("http://") {
        (80, rest)
    } else if let Some(rest) = origin.strip_prefix("https://") {
        (443, rest)
    } else {
        return false;
    };
    let (Some((origin_name, origin_port)), Some((host_name, host_port))) =
        (split_host(rest), split_host(host))
    else {
        return false;
    };
    origin_name == host_name
        && origin_port.unwrap_or(default_port) == host_port.unwrap_or(default_port)
}

/// The middleware: refuses what [`OriginGuard`] finds wrong, passes the rest.
pub async fn guard(
    State(guard): State<Arc<OriginGuard>>,
    request: Request,
    next: Next,
) -> Response {
    let uri_host = request.uri().authority().map(|a| a.as_str().to_owned());
    let verdict = guard.check(request.method(), request.headers(), uri_host.as_deref());
    let api = request.uri().path().starts_with("/api");
    match verdict {
        Verdict::Pass => next.run(request).await,
        Verdict::Host(host) => {
            guard.refused_hosts.fetch_add(1, Ordering::Relaxed);
            guard.note(Some(host.as_deref().unwrap_or("")));
            refusal(
                api,
                StatusCode::MISDIRECTED_REQUEST,
                "host_not_allowed",
                &format!(
                    "이 주소 이름으로는 trss를 열 수 없어요. IP 주소로 들어오거나, 서버의 .env에서 {HOSTS_VAR}에 이 이름을 더해 주세요."
                ),
            )
        }
        Verdict::Origin => {
            guard.refused_origins.fetch_add(1, Ordering::Relaxed);
            guard.note(None);
            refusal(
                api,
                StatusCode::FORBIDDEN,
                "forbidden",
                "다른 사이트에서 보낸 요청은 받지 않아요.",
            )
        }
    }
}

fn refusal(api: bool, status: StatusCode, code: &str, message: &str) -> Response {
    if api {
        (status, Json(json!({ "error": code, "message": message }))).into_response()
    } else {
        (
            status,
            [(
                header::CONTENT_TYPE,
                HeaderValue::from_static("text/plain; charset=utf-8"),
            )],
            format!("{message}\n"),
        )
            .into_response()
    }
}

#[cfg(test)]
mod tests {
    use axum::{body::Body, http::Request, middleware, routing::get, Router};
    use http_body_util::BodyExt;
    use tower::ServiceExt;

    use super::*;

    fn app(hosts: &str) -> Router {
        let guard = Arc::new(OriginGuard::new(AllowedHosts::parse(hosts).unwrap()));
        Router::new()
            .route(
                "/api/thing",
                get(|| async { "read" }).post(|| async { "changed" }),
            )
            .route("/ws", get(|| async { "upgraded" }))
            .fallback(|| async { "page" })
            .layer(middleware::from_fn_with_state(guard, super::guard))
    }

    async fn send(
        app: &Router,
        method: Method,
        uri: &str,
        headers: &[(&str, &str)],
    ) -> (StatusCode, String) {
        let mut request = Request::builder().method(method).uri(uri);
        for (name, value) in headers {
            request = request.header(*name, *value);
        }
        let response = app
            .clone()
            .oneshot(request.body(Body::empty()).unwrap())
            .await
            .unwrap();
        let status = response.status();
        let body = response.into_body().collect().await.unwrap().to_bytes();
        (status, String::from_utf8_lossy(&body).into_owned())
    }

    #[test]
    fn the_host_names_list() {
        let hosts =
            AllowedHosts::parse(" TRSS.example.com , media.lan:8443,, nas.local. ").unwrap();
        assert_eq!(
            hosts.names(),
            ["trss.example.com", "media.lan", "nas.local"]
        );
        assert_eq!(AllowedHosts::parse("").unwrap(), AllowedHosts::default());
        for bad in [
            "http://trss.example",
            "a b",
            "trss.example/x",
            "u@trss.example",
        ] {
            assert_eq!(AllowedHosts::parse(bad), None, "{bad}");
        }
    }

    #[test]
    fn which_hosts_are_allowed() {
        let hosts = AllowedHosts::parse("trss.example").unwrap();
        for host in [
            "192.168.1.116",
            "192.168.1.116:8080",
            "127.0.0.1:8080",
            "[::1]",
            "[::1]:8080",
            "[fe80::1]:80",
            "localhost",
            "localhost:5173",
            "LOCALHOST:8080",
            "localhost.:8080",
            "trss.example",
            "TRSS.Example:443",
            "trss.example.",
        ] {
            assert!(hosts.allows(host), "{host} refused");
        }
        for host in [
            "evil.example",
            "evil.example:8080",
            "trss.example.evil.example",
            "192.168.1.116.nip.io",
            "sub.localhost",
            "localhost.evil.example",
            "",
            "::1",
            "user@localhost",
            "localhost/x",
        ] {
            assert!(!hosts.allows(host), "{host} allowed");
        }
        assert!(!AllowedHosts::default().allows("trss.example"));
    }

    #[test]
    fn origins() {
        assert!(same_origin(
            "http://192.168.1.116:8080",
            "192.168.1.116:8080"
        ));
        assert!(same_origin("http://localhost:8080", "LOCALHOST:8080"));
        assert!(same_origin("https://trss.example", "trss.example"));
        assert!(same_origin("https://trss.example", "trss.example:443"));
        assert!(same_origin("http://trss.example", "trss.example:80"));
        assert!(same_origin("http://[::1]:8080", "[::1]:8080"));
        // The scheme is not compared (TLS may end at a reverse proxy).
        assert!(same_origin(
            "https://192.168.1.116:8080",
            "192.168.1.116:8080"
        ));
        // Without a port in `Host`, both default ports pass: the scheme the
        // browser used is not known here (a known, documented limit).
        assert!(same_origin("https://192.168.1.116", "192.168.1.116"));
        assert!(same_origin("http://192.168.1.116", "192.168.1.116"));

        assert!(!same_origin("null", "192.168.1.116:8080"));
        assert!(!same_origin(
            "http://192.168.1.116:8081",
            "192.168.1.116:8080"
        ));
        assert!(!same_origin("http://192.168.1.116", "192.168.1.116:8080"));
        assert!(!same_origin("https://trss.example", "trss.example:80"));
        assert!(!same_origin(
            "http://evil.example:8080",
            "192.168.1.116:8080"
        ));
        assert!(!same_origin("file://", "localhost"));
        assert!(!same_origin("chrome-extension://abc", "localhost"));
        assert!(!same_origin("http://localhost:8080/path", "localhost:8080"));
    }

    #[tokio::test]
    async fn a_host_that_is_not_allowed_is_refused_everywhere() {
        let app = app("trss.example");
        for (method, uri) in [
            (Method::GET, "/api/thing"),
            (Method::POST, "/api/thing"),
            (Method::GET, "/"),
            (Method::GET, "/library"),
            (Method::GET, "/ws"),
        ] {
            let (status, body) =
                send(&app, method.clone(), uri, &[("host", "evil.example:8080")]).await;
            assert_eq!(status, StatusCode::MISDIRECTED_REQUEST, "{method} {uri}");
            assert!(body.contains(HOSTS_VAR), "{body}");
            assert_eq!(
                body.contains("host_not_allowed"),
                uri.starts_with("/api"),
                "{body}"
            );
        }
        for host in [
            "192.168.1.116:8080",
            "localhost:8080",
            "trss.example",
            "[::1]:8080",
        ] {
            let (status, body) = send(&app, Method::GET, "/api/thing", &[("host", host)]).await;
            assert_eq!((status, body.as_str()), (StatusCode::OK, "read"), "{host}");
        }
        // No `Host` at all: not a browser page.
        let (status, _) = send(&app, Method::GET, "/api/thing", &[]).await;
        assert_eq!(status, StatusCode::OK);
    }

    #[tokio::test]
    async fn a_change_from_another_origin_is_refused() {
        let app = app("");
        let host = ("host", "192.168.1.116:8080");
        let refused: &[&[(&str, &str)]] = &[
            &[host, ("origin", "http://evil.example")],
            &[host, ("origin", "null")],
            &[host, ("origin", "http://192.168.1.116:9091")],
            &[
                host,
                ("origin", "http://192.168.1.116:8080"),
                ("sec-fetch-site", "cross-site"),
            ],
            &[host, ("sec-fetch-site", "cross-site")],
        ];
        for headers in refused {
            let (status, body) = send(&app, Method::POST, "/api/thing", headers).await;
            assert_eq!(status, StatusCode::FORBIDDEN, "{headers:?}");
            assert!(body.contains("\"forbidden\""), "{body}");
        }
        let passed: &[&[(&str, &str)]] = &[
            &[host, ("origin", "http://192.168.1.116:8080")],
            &[
                host,
                ("origin", "http://192.168.1.116:8080"),
                ("sec-fetch-site", "same-origin"),
            ],
            // Not from a browser page.
            &[host],
        ];
        for headers in passed {
            let (status, body) = send(&app, Method::POST, "/api/thing", headers).await;
            assert_eq!(
                (status, body.as_str()),
                (StatusCode::OK, "changed"),
                "{headers:?}"
            );
        }
        // Reading is not a change: another origin cannot read the answer.
        let (status, _) = send(
            &app,
            Method::GET,
            "/api/thing",
            &[
                host,
                ("origin", "http://evil.example"),
                ("sec-fetch-site", "cross-site"),
            ],
        )
        .await;
        assert_eq!(status, StatusCode::OK);
    }

    #[tokio::test]
    async fn a_websocket_needs_its_own_origin() {
        let app = app("");
        let host = ("host", "localhost:8080");
        let upgrade = [("connection", "Upgrade"), ("upgrade", "websocket")];
        for extra in [
            vec![],
            vec![("origin", "null")],
            vec![("origin", "http://evil.example:8080")],
            vec![("origin", "http://localhost:5173")],
            vec![
                ("origin", "http://localhost:8080"),
                ("sec-fetch-site", "cross-site"),
            ],
        ] {
            let headers: Vec<(&str, &str)> = [host]
                .into_iter()
                .chain(upgrade)
                .chain(extra.clone())
                .collect();
            let (status, _) = send(&app, Method::GET, "/ws", &headers).await;
            assert_eq!(status, StatusCode::FORBIDDEN, "{extra:?}");
        }
        let headers = [
            host,
            upgrade[0],
            upgrade[1],
            ("origin", "http://localhost:8080"),
        ];
        let (status, body) = send(&app, Method::GET, "/ws", &headers).await;
        assert_eq!((status, body.as_str()), (StatusCode::OK, "upgraded"));
        // The text answer outside /api.
        let (status, body) = send(&app, Method::GET, "/ws", &[host, upgrade[0], upgrade[1]]).await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        assert!(!body.contains('{'), "{body}");
    }
}
