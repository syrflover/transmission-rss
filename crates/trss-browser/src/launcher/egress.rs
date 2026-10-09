//! The browser's way out: a forward proxy on the container's loopback that
//! every connection of Chromium goes through.
//!
//! The server browser runs the pages of the sites it is sent to, and the
//! container's network (`browser_net`, a bridge) reaches the host's LAN
//! address, the Docker gateway and the LAN behind them. Chromium is started
//! with this proxy as its only way out ([`super::Launcher`] sets the flags), and
//! the proxy connects only to public addresses:
//!
//! - `CONNECT host:port` (HTTPS, and WebSockets of either scheme) and plain
//!   HTTP requests in absolute form (`GET http://host/path`) are served; any
//!   other request is refused.
//! - The proxy resolves the name itself and refuses the connection when any
//!   address it resolves to is not public ([`is_public`]); otherwise it
//!   connects to those very addresses, without resolving again. A name that
//!   resolves to a public address now and to a private one later (DNS
//!   rebinding) is judged on each connection by the address it is about to
//!   use. A name that mixes public and private addresses is refused: no
//!   public site needs that, and it is the shape of a rebinding attempt.
//! - Besides the address classes, the networks the container itself is on
//!   and its gateways are refused ([`LocalNetworks`], read from the routing
//!   tables when the launcher starts), so a Docker network made from a pool
//!   outside the private ranges, or an IPv6 one, is not reachable either.
//! - Neither a refused nor an allowed destination is logged. A refusal is
//!   counted, and the count is noted at most once a minute.
//!
//! What it reads from the browser is bounded (the request head, see
//! [`MAX_HEAD`] and [`HEAD_TIMEOUT`]); a tunnel copies bytes with fixed
//! buffers and holds nothing else. At most [`MAX_LOOKUPS`] names are being
//! resolved at a time.
//!
//! What it cannot tell from an address: a public address that leads back
//! into the LAN, such as the router's WAN address with hairpin NAT, or the
//! user's own domain pointing at it. Those are public addresses to the proxy.

use std::{
    convert::Infallible,
    future::Future,
    io,
    net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr},
    pin::Pin,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex,
    },
    time::{Duration, Instant},
};

use bytes::Bytes;
use http_body_util::{combinators::BoxBody, BodyExt, Full};
use hyper::{
    body::Incoming,
    header::{self, HeaderMap, HeaderName, HeaderValue},
    http::uri::{PathAndQuery, Scheme},
    service::service_fn,
    Method, Request, Response, StatusCode, Uri,
};
use hyper_util::rt::{TokioIo, TokioTimer};
use tokio::net::{TcpListener, TcpStream};
use trss_core::net_route;

/// The most a request head (request line and headers) may take.
pub const MAX_HEAD: usize = 64 * 1024;
/// How long the browser has to send a request head.
pub const HEAD_TIMEOUT: Duration = Duration::from_secs(30);
/// How long resolving a name may take, and connecting to each of its
/// addresses.
pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
/// How many names the system resolver looks up at once. Each lookup holds a
/// blocking thread until `getaddrinfo` returns, whether or not the proxy is
/// still waiting for it.
pub const MAX_LOOKUPS: usize = 8;
/// How often, at most, the count of refusals is noted in the log.
const NOTE_EVERY: Duration = Duration::from_secs(60);

/// What the browser is shown when a connection is refused.
const REFUSED_TEXT: &str = "trss: 서버 브라우저는 로컬 네트워크와 사설 주소에 연결하지 않아요.\n";

type ProxyBody = BoxBody<Bytes, hyper::Error>;

/// Resolves a name to the addresses to connect to.
pub trait Resolve: Send + Sync + 'static {
    fn resolve<'a>(
        &'a self,
        host: &'a str,
        port: u16,
    ) -> Pin<Box<dyn Future<Output = io::Result<Vec<SocketAddr>>> + Send + 'a>>;
}

/// A blocking lookup run on tokio's blocking threads, at most `limit` at a
/// time. A lookup keeps its turn until it returns, even when the proxy has
/// stopped waiting for it, so slow DNS cannot pile up blocking threads.
pub struct BlockingResolver<F> {
    lookup: Arc<F>,
    turns: Arc<tokio::sync::Semaphore>,
}

impl<F> BlockingResolver<F>
where
    F: Fn(&str, u16) -> io::Result<Vec<SocketAddr>> + Send + Sync + 'static,
{
    pub fn new(lookup: F, limit: usize) -> BlockingResolver<F> {
        BlockingResolver {
            lookup: Arc::new(lookup),
            turns: Arc::new(tokio::sync::Semaphore::new(limit)),
        }
    }
}

impl<F> Resolve for BlockingResolver<F>
where
    F: Fn(&str, u16) -> io::Result<Vec<SocketAddr>> + Send + Sync + 'static,
{
    fn resolve<'a>(
        &'a self,
        host: &'a str,
        port: u16,
    ) -> Pin<Box<dyn Future<Output = io::Result<Vec<SocketAddr>>> + Send + 'a>> {
        Box::pin(async move {
            let turn = self
                .turns
                .clone()
                .acquire_owned()
                .await
                .map_err(io::Error::other)?;
            let (lookup, host) = (self.lookup.clone(), host.to_owned());
            tokio::task::spawn_blocking(move || {
                let _turn = turn;
                lookup(&host, port)
            })
            .await
            .map_err(io::Error::other)?
        })
    }
}

/// The system's resolver (`/etc/hosts`, then the container's DNS), at most
/// [`MAX_LOOKUPS`] lookups at a time.
pub fn system_resolver() -> impl Resolve {
    BlockingResolver::new(
        |host: &str, port| {
            use std::net::ToSocketAddrs;
            Ok((host, port).to_socket_addrs()?.collect())
        },
        MAX_LOOKUPS,
    )
}

/// The networks the container is on and its gateways, which the proxy
/// refuses whatever their addresses are.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LocalNetworks {
    /// (network, prefix length); a gateway is a whole-length prefix.
    networks: Vec<(IpAddr, u8)>,
}

impl LocalNetworks {
    /// Reads the container's routing tables (`/proc/net/route` and
    /// `/proc/net/ipv6_route`). A table that cannot be read gives nothing.
    pub fn read() -> LocalNetworks {
        let read = |path| std::fs::read_to_string(path).unwrap_or_default();
        LocalNetworks::from_tables(&read(net_route::PATH), &read("/proc/net/ipv6_route"))
    }

    /// The networks of the routes in `route` and `ipv6_route` (the text of
    /// the two `/proc/net` tables) and their gateways. Routes on loopback,
    /// and routes wider than a /8 (IPv4) or a /16 (IPv6), such as the
    /// default route, give no network: they are the way to the internet, not
    /// a network the container is on.
    pub fn from_tables(route: &str, ipv6_route: &str) -> LocalNetworks {
        let mut networks = Vec::new();
        for route in net_route::parse(route) {
            if route.iface == "lo" {
                continue;
            }
            if route.prefix() >= 8 {
                networks.push((IpAddr::V4(route.destination), route.prefix()));
            }
            if !route.gateway.is_unspecified() {
                networks.push((IpAddr::V4(route.gateway), 32));
            }
        }
        // Destination PrefixLen Source SourcePrefixLen NextHop Metric RefCnt
        // Use Flags Iface; the addresses are 32 hexadecimal digits.
        for line in ipv6_route.lines() {
            let fields: Vec<&str> = line.split_whitespace().collect();
            if fields.len() < 10 || fields[9] == "lo" {
                continue;
            }
            let v6 = |hex: &str| u128::from_str_radix(hex, 16).ok().map(Ipv6Addr::from);
            let (Some(destination), Ok(prefix), Some(next_hop)) = (
                v6(fields[0]),
                u8::from_str_radix(fields[1], 16),
                v6(fields[4]),
            ) else {
                continue;
            };
            if prefix >= 16 {
                networks.push((IpAddr::V6(destination), prefix));
            }
            if !next_hop.is_unspecified() {
                networks.push((IpAddr::V6(next_hop), 128));
            }
        }
        networks.sort();
        networks.dedup();
        LocalNetworks { networks }
    }

    pub fn len(&self) -> usize {
        self.networks.len()
    }

    pub fn is_empty(&self) -> bool {
        self.networks.is_empty()
    }

    /// Whether `ip` is in one of the networks (an IPv4-mapped address as its
    /// IPv4).
    pub fn contains(&self, ip: IpAddr) -> bool {
        let ip = ip.to_canonical();
        self.networks
            .iter()
            .any(|(network, prefix)| match (network, ip) {
                (IpAddr::V4(network), IpAddr::V4(ip)) => {
                    let mask = u32::MAX.checked_shl(32 - u32::from(*prefix)).unwrap_or(0);
                    u32::from(ip) & mask == u32::from(*network) & mask
                }
                (IpAddr::V6(network), IpAddr::V6(ip)) => {
                    in_v6(u128::from(ip), u128::from(*network), u32::from(*prefix))
                }
                _ => false,
            })
    }
}

/// Why a connection was not made.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refusal {
    /// The target is not a host and port.
    BadTarget,
    /// The request is not one a proxy serves (not CONNECT, not absolute
    /// `http:`).
    NotProxied,
    /// An address the name resolves to is not public.
    NotPublic,
    /// The name resolves to nothing.
    Unresolved,
    /// No address answered.
    Unreachable,
    TimedOut,
}

impl Refusal {
    fn response(self) -> Response<ProxyBody> {
        let (status, text) = match self {
            Refusal::BadTarget | Refusal::NotProxied => {
                (StatusCode::BAD_REQUEST, "trss: 프록시 요청이 아니에요.\n")
            }
            Refusal::NotPublic => (StatusCode::FORBIDDEN, REFUSED_TEXT),
            Refusal::Unresolved => (StatusCode::BAD_GATEWAY, "trss: 주소를 찾지 못했어요.\n"),
            Refusal::Unreachable => (StatusCode::BAD_GATEWAY, "trss: 연결하지 못했어요.\n"),
            Refusal::TimedOut => (
                StatusCode::GATEWAY_TIMEOUT,
                "trss: 연결이 시간 안에 되지 않았어요.\n",
            ),
        };
        let mut response = Response::new(full(text));
        *response.status_mut() = status;
        response.headers_mut().insert(
            header::CONTENT_TYPE,
            HeaderValue::from_static("text/plain; charset=utf-8"),
        );
        response
    }
}

/// The proxy's rules and its count of refusals. Shared by its connections.
pub struct Egress {
    resolver: Box<dyn Resolve>,
    /// Addresses let through although they are not public: for tests that
    /// serve a site on the host (`TRSS_BROWSER_EGRESS_ALLOW`). Empty in
    /// deployments.
    allowed: Vec<SocketAddr>,
    /// The container's own networks and gateways, refused too.
    local: LocalNetworks,
    connect_timeout: Duration,
    refused: AtomicU64,
    last_note: Mutex<Option<Instant>>,
}

impl Egress {
    pub fn new(resolver: impl Resolve, allowed: Vec<SocketAddr>) -> Egress {
        Egress {
            resolver: Box::new(resolver),
            allowed,
            local: LocalNetworks::default(),
            connect_timeout: CONNECT_TIMEOUT,
            refused: AtomicU64::new(0),
            last_note: Mutex::new(None),
        }
    }

    /// Refuses `local` too, whatever their addresses are.
    pub fn with_local_networks(mut self, local: LocalNetworks) -> Egress {
        self.local = local;
        self
    }

    /// How long resolving, and connecting to each address, may take.
    pub fn with_connect_timeout(mut self, timeout: Duration) -> Egress {
        self.connect_timeout = timeout;
        self
    }

    /// How many connections have been refused for a destination that is not
    /// public.
    pub fn refused(&self) -> u64 {
        self.refused.load(Ordering::Relaxed)
    }

    fn permits(&self, addr: &SocketAddr) -> bool {
        (is_public(addr.ip()) && !self.local.contains(addr.ip())) || self.allowed.contains(addr)
    }

    fn count_refusal(&self) {
        let count = self.refused.fetch_add(1, Ordering::Relaxed) + 1;
        let mut last = self.last_note.lock().expect("note lock");
        if last.is_none_or(|at| at.elapsed() >= NOTE_EVERY) {
            *last = Some(Instant::now());
            // A count only: never the address, the name or the URL.
            eprintln!(
                "Browser egress: {count} connections to addresses that are not public refused so far"
            );
        }
    }

    /// Connects to `host:port` when every address it stands for is allowed.
    pub async fn connect(&self, host: &str, port: u16) -> Result<TcpStream, Refusal> {
        let addresses = match literal(host) {
            Some(ip) => vec![SocketAddr::new(ip, port)],
            None => {
                if host.is_empty() {
                    return Err(Refusal::BadTarget);
                }
                match tokio::time::timeout(self.connect_timeout, self.resolver.resolve(host, port))
                    .await
                {
                    Ok(Ok(addresses)) => addresses,
                    Ok(Err(_)) => return Err(Refusal::Unresolved),
                    Err(_) => return Err(Refusal::TimedOut),
                }
            }
        };
        if addresses.is_empty() {
            return Err(Refusal::Unresolved);
        }
        if !addresses.iter().all(|a| self.permits(a)) {
            self.count_refusal();
            return Err(Refusal::NotPublic);
        }
        // Each address has its own time, so one that drops packets does not
        // use up the time of the next.
        let mut timed_out = false;
        for address in &addresses {
            match tokio::time::timeout(self.connect_timeout, TcpStream::connect(address)).await {
                Ok(Ok(stream)) => return Ok(stream),
                Ok(Err(_)) => {}
                Err(_) => timed_out = true,
            }
        }
        Err(if timed_out {
            Refusal::TimedOut
        } else {
            Refusal::Unreachable
        })
    }

    async fn handle(self: Arc<Self>, request: Request<Incoming>) -> Response<ProxyBody> {
        let result = if request.method() == Method::CONNECT {
            self.tunnel(request).await
        } else {
            self.forward(request).await
        };
        result.unwrap_or_else(Refusal::response)
    }

    /// `CONNECT host:port`: connects, answers 200, then copies bytes both
    /// ways until either side closes.
    async fn tunnel(&self, request: Request<Incoming>) -> Result<Response<ProxyBody>, Refusal> {
        let authority = request
            .uri()
            .authority()
            .cloned()
            .ok_or(Refusal::BadTarget)?;
        let port = authority.port_u16().ok_or(Refusal::BadTarget)?;
        let mut upstream = self.connect(authority.host(), port).await?;
        tokio::spawn(async move {
            if let Ok(upgraded) = hyper::upgrade::on(request).await {
                let mut browser = TokioIo::new(upgraded);
                let _ = tokio::io::copy_bidirectional(&mut browser, &mut upstream).await;
            }
        });
        Ok(Response::new(full("")))
    }

    /// `GET http://host/path` and the like: connects, sends the request in
    /// origin form without the hop-by-hop headers, and passes the answer on.
    /// One connection to the site per request, so that a kept-alive
    /// connection from the browser never carries a request to a host that
    /// was not checked.
    async fn forward(&self, request: Request<Incoming>) -> Result<Response<ProxyBody>, Refusal> {
        let (mut parts, body) = request.into_parts();
        if parts.uri.scheme() != Some(&Scheme::HTTP) {
            return Err(Refusal::NotProxied);
        }
        let authority = parts.uri.authority().cloned().ok_or(Refusal::BadTarget)?;
        let port = authority.port_u16().unwrap_or(80);
        let upstream = self.connect(authority.host(), port).await?;

        let (mut sender, connection) =
            hyper::client::conn::http1::handshake(TokioIo::new(upstream))
                .await
                .map_err(|_| Refusal::Unreachable)?;
        tokio::spawn(connection);

        parts.uri = origin_form(&parts.uri);
        strip_hop_by_hop(&mut parts.headers);
        if !parts.headers.contains_key(header::HOST) {
            if let Ok(host) = HeaderValue::from_str(authority.as_str()) {
                parts.headers.insert(header::HOST, host);
            }
        }
        let answer = sender
            .send_request(Request::from_parts(parts, body))
            .await
            .map_err(|_| Refusal::Unreachable)?;
        let (mut parts, body) = answer.into_parts();
        strip_hop_by_hop(&mut parts.headers);
        Ok(Response::from_parts(parts, body.boxed()))
    }
}

/// Serves the proxy on `listener` until the task is dropped.
pub async fn serve(listener: TcpListener, egress: Arc<Egress>) {
    loop {
        let stream = match listener.accept().await {
            Ok((stream, _)) => stream,
            Err(_) => {
                // Out of descriptors, most likely: let some close.
                tokio::time::sleep(Duration::from_millis(100)).await;
                continue;
            }
        };
        let egress = egress.clone();
        tokio::spawn(async move {
            let service = service_fn(move |request| {
                let egress = egress.clone();
                async move { Ok::<_, Infallible>(egress.handle(request).await) }
            });
            let mut builder = hyper::server::conn::http1::Builder::new();
            builder
                .timer(TokioTimer::new())
                .header_read_timeout(HEAD_TIMEOUT)
                .max_buf_size(MAX_HEAD);
            let _ = builder
                .serve_connection(TokioIo::new(stream), service)
                .with_upgrades()
                .await;
        });
    }
}

fn full(text: &'static str) -> ProxyBody {
    Full::new(Bytes::from_static(text.as_bytes()))
        .map_err(|never| match never {})
        .boxed()
}

/// `host` as an address when it is one (`[::1]` included).
fn literal(host: &str) -> Option<IpAddr> {
    let bare = host
        .strip_prefix('[')
        .and_then(|h| h.strip_suffix(']'))
        .unwrap_or(host);
    bare.parse().ok()
}

fn origin_form(uri: &Uri) -> Uri {
    let path = uri
        .path_and_query()
        .cloned()
        .unwrap_or_else(|| PathAndQuery::from_static("/"));
    Uri::from(path)
}

/// Takes out the headers that belong to one connection (RFC 9110 7.6.1), the
/// proxy's own included.
fn strip_hop_by_hop(headers: &mut HeaderMap) {
    let named: Vec<HeaderName> = headers
        .get_all(header::CONNECTION)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(','))
        .filter_map(|name| HeaderName::from_bytes(name.trim().as_bytes()).ok())
        .collect();
    for name in named {
        headers.remove(name);
    }
    for name in [
        header::CONNECTION,
        header::PROXY_AUTHENTICATE,
        header::PROXY_AUTHORIZATION,
        header::TE,
        header::TRAILER,
        header::TRANSFER_ENCODING,
        header::UPGRADE,
    ] {
        headers.remove(name);
    }
    headers.remove("keep-alive");
    headers.remove("proxy-connection");
}

/// Whether `authority` names a host and port the proxy can be asked for.
#[cfg(test)]
fn parse_target(authority: &str) -> Option<(String, u16)> {
    let authority: hyper::http::uri::Authority = authority.parse().ok()?;
    Some((authority.host().to_owned(), authority.port_u16()?))
}

/// IPv4 blocks that are not public: this network, private, shared (CGNAT,
/// which Tailscale uses), loopback, link local, IETF protocol assignments,
/// documentation, benchmarking, multicast, reserved and broadcast.
const NOT_PUBLIC_V4: &[([u8; 4], u32)] = &[
    ([0, 0, 0, 0], 8),
    ([10, 0, 0, 0], 8),
    ([100, 64, 0, 0], 10),
    ([127, 0, 0, 0], 8),
    ([169, 254, 0, 0], 16),
    ([172, 16, 0, 0], 12),
    ([192, 0, 0, 0], 24),
    ([192, 0, 2, 0], 24),
    ([192, 168, 0, 0], 16),
    ([198, 18, 0, 0], 15),
    ([198, 51, 100, 0], 24),
    ([203, 0, 113, 0], 24),
    ([224, 0, 0, 0], 4),
    // 240.0.0.0/4 holds 255.255.255.255.
    ([240, 0, 0, 0], 4),
];

/// Blocks of global unicast (`2000::/3`) that are not public: IETF protocol
/// assignments (Teredo among them), and documentation (both blocks).
const NOT_PUBLIC_V6_GLOBAL: &[(u128, u32)] = &[
    (0x2001_0000 << 96, 23),
    (0x2001_0db8 << 96, 32),
    (0x3fff_0000 << 96, 20),
];

/// Whether the browser may connect to `ip`.
///
/// IPv4 is public outside [`NOT_PUBLIC_V4`]. IPv6 that carries an IPv4
/// address (mapped `::ffff:a.b.c.d`, translated `::ffff:0:a.b.c.d`,
/// compatible `::a.b.c.d`, which holds `::` and `::1`, and 6to4 `2002::/16`)
/// is judged by that address. Other IPv6 is public only as global unicast
/// (`2000::/3`) outside [`NOT_PUBLIC_V6_GLOBAL`]: unique local `fc00::/7`, link
/// local `fe80::/10`, multicast `ff00::/8`, NAT64 `64:ff9b::/96` and the
/// rest of the reserved space are not.
pub fn is_public(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => is_public_v4(ip),
        IpAddr::V6(ip) => is_public_v6(ip),
    }
}

fn is_public_v4(ip: Ipv4Addr) -> bool {
    let bits = u32::from(ip);
    !NOT_PUBLIC_V4.iter().any(|(net, prefix)| {
        let mask = u32::MAX << (32 - prefix);
        bits & mask == u32::from(Ipv4Addr::from(*net)) & mask
    })
}

fn in_v6(bits: u128, net: u128, prefix: u32) -> bool {
    let mask = u128::MAX << (128 - prefix);
    bits & mask == net & mask
}

fn is_public_v6(ip: Ipv6Addr) -> bool {
    let bits = u128::from(ip);
    let low32 = Ipv4Addr::from((bits & 0xffff_ffff) as u32);
    match bits >> 32 {
        // Compatible (`::/96`, with `::` and `::1`), mapped, translated.
        0 | 0xffff | 0xffff_0000 => return is_public_v4(low32),
        _ => {}
    }
    if in_v6(bits, 0x2002 << 112, 16) {
        return is_public_v4(Ipv4Addr::from(((bits >> 80) & 0xffff_ffff) as u32));
    }
    in_v6(bits, 0x2000 << 112, 3)
        && !NOT_PUBLIC_V6_GLOBAL
            .iter()
            .any(|(net, prefix)| in_v6(bits, *net, *prefix))
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    use super::*;

    #[test]
    fn the_address_table() {
        let not_public = [
            "0.0.0.0",
            "0.255.255.255",
            "10.0.0.1",
            "10.255.255.255",
            "100.64.0.1",
            "100.100.100.100",
            "100.127.255.255",
            "127.0.0.1",
            "127.255.255.254",
            "169.254.169.254",
            "172.16.0.1",
            "172.17.0.1",
            "172.31.255.255",
            "192.0.0.8",
            "192.0.2.1",
            "192.168.1.116",
            "198.18.0.1",
            "198.19.255.255",
            "198.51.100.7",
            "203.0.113.9",
            "224.0.0.251",
            "239.255.255.250",
            "240.0.0.1",
            "255.255.255.255",
            "::",
            "::1",
            "fc00::1",
            "fd12:3456::1",
            "fe80::1",
            "febf::1",
            "fec0::1",
            "ff02::1",
            "2001:db8::1",
            "2001::1",
            "2001:0:4136:e378::1",
            "3fff::1",
            "64:ff9b::808:808",
            "64:ff9b:1::1",
            "100::1",
            "::ffff:127.0.0.1",
            "::ffff:192.168.1.1",
            "::ffff:10.0.0.1",
            "::ffff:0:10.0.0.1",
            "::10.0.0.1",
            "::127.0.0.1",
            "2002:c0a8:0101::1",
            "2002:7f00:0001::",
            "2002:0a00::1",
        ];
        for ip in not_public {
            let ip: IpAddr = ip.parse().unwrap();
            assert!(!is_public(ip), "{ip} counted as public");
        }
        let public = [
            "1.1.1.1",
            "8.8.8.8",
            "9.255.255.255",
            "11.0.0.0",
            "100.63.255.255",
            "100.128.0.0",
            "126.255.255.255",
            "128.0.0.0",
            "169.253.255.255",
            "172.15.255.255",
            "172.32.0.0",
            "192.0.1.0",
            "192.167.255.255",
            "192.169.0.0",
            "198.17.255.255",
            "198.20.0.0",
            "223.255.255.255",
            "2606:4700:4700::1111",
            "2404:6800:4004:80f::200e",
            "2001:200::1",
            "::ffff:8.8.8.8",
            "::ffff:0:8.8.8.8",
            "::8.8.8.8",
            "2002:0808:0808::1",
        ];
        for ip in public {
            let ip: IpAddr = ip.parse().unwrap();
            assert!(is_public(ip), "{ip} counted as not public");
        }
    }

    #[test]
    fn targets_and_literals() {
        assert_eq!(
            parse_target("example.com:443"),
            Some(("example.com".into(), 443))
        );
        assert_eq!(parse_target("[::1]:443"), Some(("[::1]".into(), 443)));
        assert_eq!(parse_target("example.com"), None);
        assert_eq!(literal("[::1]"), Some("::1".parse().unwrap()));
        assert_eq!(
            literal("192.168.1.116"),
            Some("192.168.1.116".parse().unwrap())
        );
        assert_eq!(literal("example.com"), None);
    }

    /// Names, and what they resolve to, one answer after another.
    #[derive(Clone, Default)]
    struct Names(Arc<Mutex<HashMap<String, Vec<Vec<IpAddr>>>>>);

    impl Names {
        fn with(self, name: &str, answers: &[&[&str]]) -> Names {
            self.0.lock().unwrap().insert(
                name.to_owned(),
                answers
                    .iter()
                    .map(|a| a.iter().map(|ip| ip.parse().unwrap()).collect())
                    .collect(),
            );
            self
        }
    }

    impl Resolve for Names {
        fn resolve<'a>(
            &'a self,
            host: &'a str,
            port: u16,
        ) -> Pin<Box<dyn Future<Output = io::Result<Vec<SocketAddr>>> + Send + 'a>> {
            Box::pin(async move {
                let mut names = self.0.lock().unwrap();
                let answers = names
                    .get_mut(host)
                    .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no such name"))?;
                let answer = if answers.len() > 1 {
                    answers.remove(0)
                } else {
                    answers[0].clone()
                };
                Ok(answer
                    .into_iter()
                    .map(|ip| SocketAddr::new(ip, port))
                    .collect())
            })
        }
    }

    /// A site on loopback that counts the connections it is given and
    /// answers each request with `site` and the request line it saw.
    async fn site() -> (SocketAddr, Arc<AtomicU64>) {
        let seen = Arc::new(AtomicU64::new(0));
        let counter = seen.clone();
        let served = trss_core::loopback::serve(|listener| async move {
            loop {
                let (mut socket, _) = listener.accept().await.unwrap();
                counter.fetch_add(1, Ordering::SeqCst);
                tokio::spawn(async move {
                    let mut head = Vec::new();
                    let mut byte = [0u8; 1];
                    while !head.ends_with(b"\r\n\r\n") {
                        if socket.read(&mut byte).await.unwrap_or(0) == 0 {
                            return;
                        }
                        head.push(byte[0]);
                    }
                    let head = String::from_utf8_lossy(&head).into_owned();
                    let line = head.lines().next().unwrap_or("").to_owned();
                    let body = format!("site {line}|{}", head.to_lowercase().contains("proxy-"));
                    let answer = format!(
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                        body.len()
                    );
                    let _ = socket.write_all(answer.as_bytes()).await;
                });
            }
        })
        .await;
        (served.addr, seen)
    }

    async fn proxy(egress: Egress) -> (SocketAddr, Arc<Egress>) {
        let egress = Arc::new(egress);
        let served = trss_core::loopback::serve({
            let egress = egress.clone();
            |listener| serve(listener, egress)
        })
        .await;
        (served.addr, egress)
    }

    /// Sends `head` to the proxy and reads one answer: up to the end of the
    /// body its `Content-Length` announces, or until the connection closes
    /// or a second passes.
    async fn exchange(proxy: SocketAddr, head: &str) -> String {
        let mut socket = TcpStream::connect(proxy).await.unwrap();
        socket.write_all(head.as_bytes()).await.unwrap();
        let mut answer = Vec::new();
        let read = async {
            let mut chunk = [0u8; 4096];
            loop {
                let n = socket.read(&mut chunk).await.unwrap_or(0);
                if n == 0 {
                    return;
                }
                answer.extend_from_slice(&chunk[..n]);
                let text = String::from_utf8_lossy(&answer);
                if let Some((head, body)) = text.split_once("\r\n\r\n") {
                    let length = head.lines().find_map(|line| {
                        let (name, value) = line.split_once(':')?;
                        name.eq_ignore_ascii_case("content-length")
                            .then(|| value.trim().parse::<usize>().ok())?
                    });
                    if length.is_some_and(|length| body.len() >= length) {
                        return;
                    }
                }
            }
        };
        let _ = tokio::time::timeout(Duration::from_secs(1), read).await;
        String::from_utf8_lossy(&answer).into_owned()
    }

    #[tokio::test]
    async fn private_targets_are_refused_for_connect_and_plain_http() {
        let (site, seen) = site().await;
        let names = Names::default()
            .with("lan.example", &[&["192.168.1.116"]])
            .with("loop.example", &[&["127.0.0.1"]])
            .with("mixed.example", &[&["93.184.215.14", "127.0.0.1"]])
            .with("v6.example", &[&["::ffff:127.0.0.1"]])
            .with("localhost.", &[&["127.0.0.1", "::1"]]);
        let (proxy, egress) = proxy(Egress::new(names, Vec::new())).await;
        let port = site.port();
        let heads = [
            format!("CONNECT 127.0.0.1:{port} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\n\r\n"),
            format!("CONNECT [::1]:{port} HTTP/1.1\r\nHost: [::1]:{port}\r\n\r\n"),
            format!("CONNECT loop.example:{port} HTTP/1.1\r\nHost: loop.example\r\n\r\n"),
            format!("CONNECT mixed.example:{port} HTTP/1.1\r\nHost: mixed.example\r\n\r\n"),
            format!("CONNECT v6.example:{port} HTTP/1.1\r\nHost: v6.example\r\n\r\n"),
            "CONNECT lan.example:443 HTTP/1.1\r\nHost: lan.example:443\r\n\r\n".to_owned(),
            format!("GET http://127.0.0.1:{port}/x HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\n\r\n"),
            format!("GET http://loop.example:{port}/x HTTP/1.1\r\nHost: loop.example\r\n\r\n"),
            format!("POST http://localhost.:{port}/x HTTP/1.1\r\nHost: localhost.\r\nContent-Length: 0\r\n\r\n"),
            "GET http://lan.example/ HTTP/1.1\r\nHost: lan.example\r\n\r\n".to_owned(),
        ];
        for head in &heads {
            let answer = exchange(proxy, head).await;
            assert!(
                answer.starts_with("HTTP/1.1 403"),
                "{head:?} answered {answer:?}"
            );
        }
        assert_eq!(egress.refused(), heads.len() as u64);
        assert_eq!(seen.load(Ordering::SeqCst), 0, "the site was reached");
    }

    #[tokio::test]
    async fn a_name_that_turns_private_is_refused_on_the_connection_that_would_use_it() {
        let (site, seen) = site().await;
        // First a public address (allowed here by being listed as the site),
        // then loopback on another port.
        let names = Names::default().with(
            "rebind.example",
            &[&[&site.ip().to_string()], &["127.0.0.2"]],
        );
        let (proxy, _) = proxy(Egress::new(names, vec![site])).await;
        let head = format!(
            "GET http://rebind.example:{}/first HTTP/1.1\r\nHost: rebind.example\r\n\r\n",
            site.port()
        );
        let first = exchange(proxy, &head).await;
        assert!(first.contains("site GET /first HTTP/1.1"), "{first}");
        let second = exchange(proxy, &head).await;
        assert!(second.starts_with("HTTP/1.1 403"), "{second}");
        assert_eq!(seen.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn an_allowed_target_is_tunnelled_and_forwarded() {
        let (site, seen) = site().await;
        let names = Names::default().with("site.example", &[&["127.0.0.1"]]);
        let (proxy, _) = proxy(Egress::new(names, vec![site])).await;
        let port = site.port();

        // Plain HTTP: origin form, the proxy's headers gone.
        let answer = exchange(
            proxy,
            &format!(
                "GET http://site.example:{port}/a?b=1 HTTP/1.1\r\nHost: site.example:{port}\r\nProxy-Connection: keep-alive\r\nProxy-Authorization: x\r\n\r\n"
            ),
        )
        .await;
        assert!(answer.starts_with("HTTP/1.1 200"), "{answer}");
        assert!(
            answer.contains("site GET /a?b=1 HTTP/1.1|false"),
            "{answer}"
        );

        // CONNECT: 200, then the bytes go through as they are.
        let mut socket = TcpStream::connect(proxy).await.unwrap();
        socket
            .write_all(
                format!("CONNECT 127.0.0.1:{port} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\n\r\n")
                    .as_bytes(),
            )
            .await
            .unwrap();
        let mut head = Vec::new();
        while !head.ends_with(b"\r\n\r\n") {
            let mut byte = [0u8; 1];
            socket.read_exact(&mut byte).await.unwrap();
            head.push(byte[0]);
        }
        assert!(head.starts_with(b"HTTP/1.1 200"), "{head:?}");
        socket
            .write_all(b"GET /tunnelled HTTP/1.1\r\nHost: x\r\n\r\n")
            .await
            .unwrap();
        let mut answer = Vec::new();
        let _ = tokio::time::timeout(Duration::from_secs(1), socket.read_to_end(&mut answer)).await;
        let answer = String::from_utf8_lossy(&answer);
        assert!(answer.contains("site GET /tunnelled HTTP/1.1"), "{answer}");
        assert_eq!(seen.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn what_is_not_a_proxy_request_is_refused() {
        let (proxy, egress) = proxy(Egress::new(Names::default(), Vec::new())).await;
        for head in [
            "GET / HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n",
            "GET https://example.com/ HTTP/1.1\r\nHost: example.com\r\n\r\n",
            "CONNECT example.com HTTP/1.1\r\nHost: example.com\r\n\r\n",
        ] {
            let answer = exchange(proxy, head).await;
            assert!(
                answer.starts_with("HTTP/1.1 400"),
                "{head:?} answered {answer:?}"
            );
        }
        assert_eq!(egress.refused(), 0);
    }

    #[tokio::test]
    async fn an_oversized_head_is_refused() {
        let (proxy, _) = proxy(Egress::new(Names::default(), Vec::new())).await;
        let head = format!(
            "GET http://example.com/ HTTP/1.1\r\nHost: example.com\r\nX-Big: {}\r\n\r\n",
            "a".repeat(MAX_HEAD * 2)
        );
        let answer = exchange(proxy, &head).await;
        assert!(
            answer.is_empty() || answer.starts_with("HTTP/1.1 431"),
            "{answer:.200}"
        );
    }

    /// `/proc/net/route` and `/proc/net/ipv6_route` of a container on a
    /// network from a pool outside the private ranges, with IPv6.
    const ROUTE: &str =
        "Iface\tDestination\tGateway \tFlags\tRefCnt\tUse\tMetric\tMask\t\tMTU\tWindow\tIRTT\n\
        eth0\t00000000\t0135FF0B\t0003\t0\t0\t0\t00000000\t0\t0\t0\n\
        eth0\t0035FF0B\t00000000\t0001\t0\t0\t0\t00FFFFFF\t0\t0\t0\n\
        eth1\t000011AC\t00000000\t0001\t0\t0\t0\t0000FFFF\t0\t0\t0\n";
    const IPV6_ROUTE: &str = "\
        20010db8abcd00010000000000000000 40 00000000000000000000000000000000 00 00000000000000000000000000000000 00000100 00000001 00000000 00000001 eth0\n\
        fe800000000000000000000000000000 40 00000000000000000000000000000000 00 00000000000000000000000000000000 00000100 00000001 00000000 00000001 eth0\n\
        00000000000000000000000000000000 00 00000000000000000000000000000000 00 2a0100000000000000000000000000fe 00000400 00000001 00000000 00000003 eth0\n\
        00000000000000000000000000000001 80 00000000000000000000000000000000 00 00000000000000000000000000000000 00000000 00000002 00000000 80200001 lo\n";

    #[test]
    fn the_containers_networks_and_gateways_from_its_routing_tables() {
        let local = LocalNetworks::from_tables(ROUTE, IPV6_ROUTE);
        for ip in [
            "11.255.53.1",
            "11.255.53.200",
            "172.17.4.5",
            "2001:db8:abcd:1::7",
            "fe80::1",
            "2a01::fe",
            "::ffff:11.255.53.9",
        ] {
            assert!(local.contains(ip.parse().unwrap()), "{ip} not local");
        }
        for ip in [
            "11.255.54.1",
            "8.8.8.8",
            "2a01::ff",
            "2001:db8:abcd:2::1",
            "::1",
        ] {
            assert!(!local.contains(ip.parse().unwrap()), "{ip} local");
        }
        // The default routes give their gateways, not the whole internet.
        assert_eq!(local.len(), 6);
        assert!(LocalNetworks::from_tables("", "").is_empty());
        assert!(LocalNetworks::from_tables("garbage\nmore garbage", "x y z").is_empty());
    }

    #[tokio::test]
    async fn the_containers_own_networks_are_refused_even_when_public() {
        let names = Names::default().with("pool.example", &[&["11.255.53.7"]]);
        let egress = Egress::new(names, Vec::new())
            .with_local_networks(LocalNetworks::from_tables(ROUTE, IPV6_ROUTE));
        for (host, port) in [
            ("11.255.53.1", 80),
            ("pool.example", 443),
            ("[2001:db8:abcd:1::7]", 443),
            ("[2a01::fe]", 80),
        ] {
            assert_eq!(
                egress.connect(host, port).await.err(),
                Some(Refusal::NotPublic),
                "{host}"
            );
        }
        assert_eq!(egress.refused(), 4);
    }

    #[tokio::test]
    async fn a_name_that_resolves_to_nothing_and_an_address_that_does_not_answer() {
        let closed = trss_core::loopback::unused_addr();
        let names = Names::default().with("closed.example", &[&["127.0.0.1"]]);
        let egress = Egress::new(names, vec![closed]);
        assert_eq!(
            egress.connect("unknown.example", 443).await.err(),
            Some(Refusal::Unresolved)
        );
        assert_eq!(
            egress.connect("closed.example", closed.port()).await.err(),
            Some(Refusal::Unreachable)
        );
        let empty = Egress::new(
            BlockingResolver::new(|_: &str, _| Ok(Vec::new()), 1),
            vec![],
        );
        assert_eq!(
            empty.connect("empty.example", 443).await.err(),
            Some(Refusal::Unresolved)
        );
        assert_eq!(egress.refused() + empty.refused(), 0);
    }

    #[tokio::test]
    async fn a_lookup_that_does_not_end_times_out() {
        struct Never;
        impl Resolve for Never {
            fn resolve<'a>(
                &'a self,
                _: &'a str,
                _: u16,
            ) -> Pin<Box<dyn Future<Output = io::Result<Vec<SocketAddr>>> + Send + 'a>>
            {
                Box::pin(std::future::pending())
            }
        }
        let egress =
            Egress::new(Never, Vec::new()).with_connect_timeout(Duration::from_millis(100));
        let began = Instant::now();
        assert_eq!(
            egress.connect("slow.example", 443).await.err(),
            Some(Refusal::TimedOut)
        );
        assert!(began.elapsed() < Duration::from_secs(2));
    }

    /// An address that takes no connection (a listener whose queue is full
    /// drops the handshake) does not use up the time of the next address.
    #[tokio::test]
    async fn each_address_has_its_own_connect_time() {
        let full = tokio::net::TcpSocket::new_v4().unwrap();
        full.bind("127.0.0.1:0".parse().unwrap()).unwrap();
        let full = full.listen(0).unwrap();
        let hanging = full.local_addr().unwrap();
        // Fill the queue: these are never accepted.
        let mut queued = Vec::new();
        for _ in 0..4 {
            if let Ok(Ok(stream)) =
                tokio::time::timeout(Duration::from_millis(200), TcpStream::connect(hanging)).await
            {
                queued.push(stream);
            }
        }
        let (site, seen) = site().await;
        let both = vec![hanging, site];
        let resolver = BlockingResolver::new(move |_: &str, _| Ok(both.clone()), 1);
        let egress = Egress::new(resolver, vec![hanging, site])
            .with_connect_timeout(Duration::from_millis(300));
        let began = Instant::now();
        let stream = egress.connect("two.example", 80).await;
        assert!(stream.is_ok(), "{:?}", stream.err());
        assert!(began.elapsed() < Duration::from_secs(2));
        drop(stream);
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert_eq!(seen.load(Ordering::SeqCst), 1);

        // Alone, the address that does not answer times out.
        let egress = Egress::new(Names::default(), vec![hanging])
            .with_connect_timeout(Duration::from_millis(300));
        assert_eq!(
            egress
                .connect(&hanging.ip().to_string(), hanging.port())
                .await
                .err(),
            Some(Refusal::TimedOut)
        );
        drop(queued);
    }

    /// A lookup keeps its turn until it returns, also when the one waiting
    /// for it gave up, and no more than the limit run at once.
    #[tokio::test]
    async fn lookups_are_bounded() {
        let running = Arc::new(AtomicU64::new(0));
        let most = Arc::new(AtomicU64::new(0));
        let (now, top) = (running.clone(), most.clone());
        let resolver = Arc::new(BlockingResolver::new(
            move |_: &str, port| {
                let at = now.fetch_add(1, Ordering::SeqCst) + 1;
                top.fetch_max(at, Ordering::SeqCst);
                std::thread::sleep(Duration::from_millis(300));
                now.fetch_sub(1, Ordering::SeqCst);
                Ok(vec![SocketAddr::from(([1, 1, 1, 1], port))])
            },
            2,
        ));
        // Two that are given up at once still hold both turns.
        for _ in 0..2 {
            let gave_up =
                tokio::time::timeout(Duration::from_millis(20), resolver.resolve("a", 1)).await;
            assert!(gave_up.is_err());
        }
        let began = Instant::now();
        let waited = resolver.resolve("b", 1).await.unwrap();
        assert_eq!(waited, [SocketAddr::from(([1, 1, 1, 1], 1))]);
        assert!(
            began.elapsed() >= Duration::from_millis(200),
            "{:?}",
            began.elapsed()
        );

        let lookups: Vec<_> = (0..6)
            .map(|_| {
                let resolver = resolver.clone();
                tokio::spawn(async move { resolver.resolve("c", 1).await })
            })
            .collect();
        for lookup in lookups {
            lookup.await.unwrap().unwrap();
        }
        assert_eq!(most.load(Ordering::SeqCst), 2);
    }
}
