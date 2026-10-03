//! The server browser's network (`browser_net`), which the web joins to reach
//! the browser's launcher for the remote screens ([`crate::screen_api`]).
//!
//! The browser runs the pages of the sites it is sent to, and the web has no
//! sign-in: a page must not reach the web. So a web that reaches the browser
//! refuses every connection whose peer address is in the subnet of its own
//! interface on that network, before any HTTP (or WebSocket) is read
//! ([`GuardedListener`]). The subnet is found at the start ([`find`]): the
//! launcher's host (`TRSS_BROWSER_URL`) is resolved, and the directly
//! connected route of this host that holds its address is the subnet
//! (`/proc/net/route`). A subnet that cannot be found stops the web from
//! starting (fail closed), and so does a launcher with an IPv6 address, which
//! the IPv4 routing table cannot place.
//!
//! The port the web publishes on the LAN reaches it through another network
//! (`trss_net`, whose gateway Docker makes the web's default by `gw_priority`
//! in `docker-compose.trss.yml`); a page that goes to the host's published
//! port arrives from that network's gateway and is not refused here. Docker
//! takes a published port through the network of the container's default
//! gateway, so a web whose default route goes through the browser's network
//! would refuse every person on the LAN: that, too, stops the start, with
//! what to change ([`check_default_route`]).
//!
//! The refusals are logged as counts, at most once a minute
//! ([`RefusalLog`]).

use std::{
    io,
    net::{IpAddr, Ipv4Addr, SocketAddr},
    time::{Duration, Instant},
};

use tokio::net::{TcpListener, TcpStream};
use url::Url;

/// An IPv4 subnet: an address and the length of its prefix.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Subnet {
    network: Ipv4Addr,
    prefix: u8,
}

impl Subnet {
    pub fn new(address: Ipv4Addr, prefix: u8) -> Subnet {
        let prefix = prefix.min(32);
        Subnet {
            network: Ipv4Addr::from(u32::from(address) & mask(prefix)),
            prefix,
        }
    }

    /// Whether `ip` is in the subnet; an IPv4 address written as IPv6
    /// (`::ffff:a.b.c.d`, how a socket bound to `::` sees it) counts as itself.
    pub fn contains(&self, ip: IpAddr) -> bool {
        match ip.to_canonical() {
            IpAddr::V4(v4) => u32::from(v4) & mask(self.prefix) == u32::from(self.network),
            IpAddr::V6(_) => false,
        }
    }
}

impl std::fmt::Display for Subnet {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}/{}", self.network, self.prefix)
    }
}

fn mask(prefix: u8) -> u32 {
    match prefix {
        0 => 0,
        p => u32::MAX << (32 - u32::from(p.min(32))),
    }
}

/// The subnet of the directly connected route (no gateway) in `routes` (the
/// text of `/proc/net/route`) that holds `ip`, the longest such prefix. A
/// default route is not one.
pub fn subnet_of(routes: &str, ip: Ipv4Addr) -> Option<Subnet> {
    // The kernel writes each address (in network order in memory) as the
    // hexadecimal of a native 32-bit number: its native bytes are the octets.
    let addr = |hex: &str| {
        u32::from_str_radix(hex, 16)
            .ok()
            .map(|v| Ipv4Addr::from(v.to_ne_bytes()))
    };
    routes
        .lines()
        .skip(1)
        .filter_map(|line| {
            let fields: Vec<&str> = line.split_whitespace().collect();
            let (destination, gateway, netmask) = (
                addr(fields.get(1)?)?,
                addr(fields.get(2)?)?,
                addr(fields.get(7)?)?,
            );
            let prefix = u32::from(netmask).count_ones() as u8;
            let subnet = Subnet::new(destination, prefix);
            (gateway.is_unspecified() && prefix > 0 && subnet.contains(IpAddr::V4(ip)))
                .then_some(subnet)
        })
        .max_by_key(|subnet| subnet.prefix)
}

/// The gateway of the default route in `routes` (the text of
/// `/proc/net/route`): of the lowest metric when there are several.
pub fn default_gateway(routes: &str) -> Option<Ipv4Addr> {
    let addr = |hex: &str| {
        u32::from_str_radix(hex, 16)
            .ok()
            .map(|v| Ipv4Addr::from(v.to_ne_bytes()))
    };
    routes
        .lines()
        .skip(1)
        .filter_map(|line| {
            let fields: Vec<&str> = line.split_whitespace().collect();
            let destination = addr(fields.get(1)?)?;
            let gateway = addr(fields.get(2)?)?;
            let metric: u32 = fields.get(6)?.parse().ok()?;
            let netmask = addr(fields.get(7)?)?;
            (destination.is_unspecified() && netmask.is_unspecified()).then_some((metric, gateway))
        })
        .min_by_key(|(metric, _)| *metric)
        .map(|(_, gateway)| gateway)
}

/// Refuses a default route through `refused`: the port the web publishes
/// would come in from that network's gateway, which the web refuses, so
/// nobody on the LAN could reach it. The error says what to change.
pub fn check_default_route(routes: &str, refused: Subnet) -> Result<(), String> {
    match default_gateway(routes) {
        Some(gateway) if refused.contains(IpAddr::V4(gateway)) => Err(format!(
            "this host's default route goes through the server browser's network {refused} \
             (gateway {gateway}), so the port published on the LAN would come in from an \
             address the web refuses. Give the web's other network the default gateway: \
             `gw_priority: 1` on trss_net in docker-compose.trss.yml, which needs Docker \
             Engine 28 or later and Compose 2.33 or later"
        )),
        _ => Ok(()),
    }
}

/// How many times the launcher's host is resolved before the web gives up:
/// the browser's container may start after the web's.
const RESOLVE_TRIES: u32 = 30;
const RESOLVE_WAIT: Duration = Duration::from_secs(1);

/// The subnet of the browser's network as this host sees it (see the module
/// docs). The error says why it was not found, in a sentence for the log.
pub async fn find(launcher: &Url) -> Result<Subnet, String> {
    let host = launcher
        .host_str()
        .ok_or_else(|| format!("the server browser's address {launcher} has no host"))?
        .trim_start_matches('[')
        .trim_end_matches(']')
        .to_owned();
    let port = launcher.port_or_known_default().unwrap_or(80);
    let mut tries = 0;
    let addresses: Vec<IpAddr> = loop {
        tries += 1;
        match tokio::net::lookup_host((host.as_str(), port)).await {
            Ok(found) => break found.map(|a| a.ip().to_canonical()).collect(),
            Err(err) if tries >= RESOLVE_TRIES => {
                return Err(format!(
                    "cannot resolve the server browser's host {host:?} ({err}), so its network is unknown"
                ))
            }
            Err(_) => tokio::time::sleep(RESOLVE_WAIT).await,
        }
    };
    if addresses.iter().any(IpAddr::is_ipv6) {
        return Err(format!(
            "the server browser's host {host:?} has an IPv6 address; only an IPv4 network can be refused"
        ));
    }
    let routes = tokio::fs::read_to_string("/proc/net/route")
        .await
        .map_err(|e| format!("cannot read this host's routes (/proc/net/route): {e}"))?;
    let mut subnets = addresses.iter().filter_map(|ip| match ip {
        IpAddr::V4(v4) => Some((v4, subnet_of(&routes, *v4))),
        IpAddr::V6(_) => None,
    });
    match subnets.next() {
        Some((_, Some(subnet))) if subnets.all(|(_, other)| other == Some(subnet)) => {
            check_default_route(&routes, subnet).map(|()| subnet)
        }
        Some((ip, None)) => Err(format!(
            "the server browser's address {ip} is on no network of this host's own"
        )),
        Some(_) => Err(format!(
            "the server browser's host {host:?} is on more than one network of this host's"
        )),
        None => Err(format!("the server browser's host {host:?} has no address")),
    }
}

/// How often the refusals are logged at most.
pub const LOG_EVERY: Duration = Duration::from_secs(60);

/// The refusals not logged yet: logged as a count at most every
/// [`LOG_EVERY`], so a page that keeps trying cannot fill the log.
#[derive(Debug, Default)]
pub struct RefusalLog {
    unlogged: u64,
    last: Option<Instant>,
}

impl RefusalLog {
    /// One more refusal at `now`; the count to log now, if it is time.
    pub fn refused(&mut self, now: Instant) -> Option<u64> {
        self.unlogged += 1;
        let due = self
            .last
            .is_none_or(|at| now.duration_since(at) >= LOG_EVERY);
        if !due {
            return None;
        }
        self.last = Some(now);
        Some(std::mem::take(&mut self.unlogged))
    }
}

/// A TCP listener that drops every connection from `refused` as soon as it is
/// accepted, before anything is read from it.
pub struct GuardedListener {
    inner: TcpListener,
    refused: Option<Subnet>,
    log: RefusalLog,
}

impl GuardedListener {
    /// `refused`: the browser's network; `None` refuses nothing (a web with no
    /// server browser).
    pub fn new(inner: TcpListener, refused: Option<Subnet>) -> GuardedListener {
        GuardedListener {
            inner,
            refused,
            log: RefusalLog::default(),
        }
    }

    /// Whether a connection from `peer` is refused.
    pub fn refuses(&self, peer: &SocketAddr) -> bool {
        self.refused.is_some_and(|net| net.contains(peer.ip()))
    }
}

impl axum::serve::Listener for GuardedListener {
    type Io = TcpStream;
    type Addr = SocketAddr;

    async fn accept(&mut self) -> (TcpStream, SocketAddr) {
        loop {
            let (stream, peer) = axum::serve::Listener::accept(&mut self.inner).await;
            if self.refuses(&peer) {
                // Closed unread, and counted.
                drop(stream);
                if let Some(count) = self.log.refused(Instant::now()) {
                    let net = self.refused.map(|n| n.to_string()).unwrap_or_default();
                    eprintln!(
                        "trss-web: refused {count} connection(s) from the server browser's network ({net}) since the last report"
                    );
                }
                continue;
            }
            return (stream, peer);
        }
    }

    fn local_addr(&self) -> io::Result<SocketAddr> {
        self.inner.local_addr()
    }
}

#[cfg(test)]
mod tests {
    use axum::{routing::get, Router};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    use super::*;

    /// A routing table of a container on two bridge networks, as a
    /// little-endian host writes it:
    /// 172.22.0.0/16 (eth0, the default route's) and 172.23.0.0/16 (eth1).
    const ROUTES: &str =
        "Iface\tDestination\tGateway \tFlags\tRefCnt\tUse\tMetric\tMask\t\tMTU\tWindow\tIRTT
eth0\t00000000\t010016AC\t0003\t0\t0\t0\t00000000\t0\t0\t0
eth0\t000016AC\t00000000\t0001\t0\t0\t0\t0000FFFF\t0\t0\t0
eth1\t000017AC\t00000000\t0001\t0\t0\t0\t0000FFFF\t0\t0\t0
eth1\t000A17AC\t00000000\t0001\t0\t0\t0\t00FFFFFF\t0\t0\t0
";

    #[test]
    fn the_browsers_subnet_is_the_connected_route_that_holds_it() {
        let ip = Ipv4Addr::new(172, 23, 0, 3);
        assert_eq!(
            subnet_of(ROUTES, ip),
            Some(Subnet::new(Ipv4Addr::new(172, 23, 0, 0), 16))
        );
        // The longest prefix wins.
        assert_eq!(
            subnet_of(ROUTES, Ipv4Addr::new(172, 23, 10, 9)),
            Some(Subnet::new(Ipv4Addr::new(172, 23, 10, 0), 24))
        );
        // Only through a gateway (the default route): not a network of ours.
        assert_eq!(subnet_of(ROUTES, Ipv4Addr::new(8, 8, 8, 8)), None);
    }

    #[test]
    fn a_default_route_through_the_browsers_network_stops_the_start() {
        let browser_net = Subnet::new(Ipv4Addr::new(172, 23, 0, 0), 16);
        // The default route goes through 172.22.0.1 (trss_net): fine.
        assert_eq!(default_gateway(ROUTES), Some(Ipv4Addr::new(172, 22, 0, 1)));
        assert_eq!(check_default_route(ROUTES, browser_net), Ok(()));
        // Without `gw_priority` Docker may give browser_net the default
        // gateway (172.23.0.1).
        let through_browser =
            "Iface\tDestination\tGateway \tFlags\tRefCnt\tUse\tMetric\tMask\t\tMTU\tWindow\tIRTT
eth1\t00000000\t010017AC\t0003\t0\t0\t0\t00000000\t0\t0\t0
eth0\t000016AC\t00000000\t0001\t0\t0\t0\t0000FFFF\t0\t0\t0
eth1\t000017AC\t00000000\t0001\t0\t0\t0\t0000FFFF\t0\t0\t0
";
        let err = check_default_route(through_browser, browser_net).unwrap_err();
        assert!(err.contains("gw_priority"), "{err}");
        assert!(
            err.contains("Engine 28") && err.contains("Compose 2.33"),
            "{err}"
        );
        // Of several default routes, the one of the lowest metric counts.
        let defaults = |browser_metric: u32, trss_metric: u32| {
            format!(
                "Iface\tDestination\tGateway \tFlags\tRefCnt\tUse\tMetric\tMask\t\tMTU\tWindow\tIRTT
eth1\t00000000\t010017AC\t0003\t0\t0\t{browser_metric}\t00000000\t0\t0\t0
eth0\t00000000\t010016AC\t0003\t0\t0\t{trss_metric}\t00000000\t0\t0\t0
"
            )
        };
        assert!(check_default_route(&defaults(0, 5), browser_net).is_err());
        let preferred = defaults(9, 5);
        assert_eq!(
            default_gateway(&preferred),
            Some(Ipv4Addr::new(172, 22, 0, 1))
        );
        assert_eq!(check_default_route(&preferred, browser_net), Ok(()));
        // No default route at all: nothing comes in through a gateway.
        let none =
            "Iface\tDestination\tGateway \tFlags\tRefCnt\tUse\tMetric\tMask\t\tMTU\tWindow\tIRTT
eth1\t000017AC\t00000000\t0001\t0\t0\t0\t0000FFFF\t0\t0\t0
";
        assert_eq!(check_default_route(none, browser_net), Ok(()));
    }

    #[test]
    fn refusals_are_logged_as_counts_at_most_once_a_minute() {
        let mut log = RefusalLog::default();
        let start = Instant::now();
        // The first one at once, then none for a minute.
        assert_eq!(log.refused(start), Some(1));
        for n in 1..=50 {
            assert_eq!(log.refused(start + Duration::from_secs(n)), None);
        }
        assert_eq!(log.refused(start + LOG_EVERY), Some(51));
        assert_eq!(
            log.refused(start + LOG_EVERY + Duration::from_secs(1)),
            None
        );
        assert_eq!(log.refused(start + LOG_EVERY * 3), Some(2));
    }

    #[test]
    fn a_subnet_holds_its_addresses_also_written_as_ipv6() {
        let net = Subnet::new(Ipv4Addr::new(172, 23, 9, 9), 16);
        assert_eq!(net.to_string(), "172.23.0.0/16");
        assert!(net.contains("172.23.0.1".parse().unwrap()));
        assert!(net.contains("::ffff:172.23.255.3".parse().unwrap()));
        assert!(!net.contains("172.22.0.1".parse().unwrap()));
        assert!(!net.contains("::1".parse().unwrap()));
    }

    #[tokio::test]
    async fn a_launcher_on_no_network_of_this_host_stops_the_web() {
        // Loopback is no route of the main table: fail closed.
        let err = find(&Url::parse("http://127.0.0.1:9230").unwrap())
            .await
            .unwrap_err();
        assert!(err.contains("on no network"), "{err}");
    }

    /// Serves `/` on loopback behind a listener that refuses `refused`, and
    /// asks it once: the answer, or nothing when the connection was closed.
    async fn ask(refused: Option<Subnet>) -> String {
        let inner = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = inner.local_addr().unwrap();
        let app = Router::new().route("/", get(|| async { "hello" }));
        let server = tokio::spawn(async move {
            axum::serve(GuardedListener::new(inner, refused), app)
                .await
                .unwrap()
        });
        let mut stream = TcpStream::connect(addr).await.unwrap();
        // The write may fail on a closed connection; the read says the rest.
        let _ = stream
            .write_all(b"GET / HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n")
            .await;
        let mut answer = Vec::new();
        let _ = tokio::time::timeout(Duration::from_secs(5), stream.read_to_end(&mut answer)).await;
        server.abort();
        String::from_utf8_lossy(&answer).into_owned()
    }

    #[tokio::test]
    async fn a_peer_in_the_browsers_network_is_refused_before_anything_is_read() {
        // The peer is 127.0.0.1: here the browser's network is loopback's.
        let refused = ask(Some(Subnet::new(Ipv4Addr::LOCALHOST, 8))).await;
        assert_eq!(refused, "");
        let served = ask(Some(Subnet::new(Ipv4Addr::new(172, 23, 0, 0), 16))).await;
        assert!(served.starts_with("HTTP/1.1 200"), "{served}");
        assert!(served.ends_with("hello"), "{served}");
        assert!(ask(None).await.ends_with("hello"));
    }
}
