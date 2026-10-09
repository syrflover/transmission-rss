//! The host's IPv4 routing table (`/proc/net/route`) as routes.
//!
//! The web finds the subnet of the server browser's network and the default
//! gateway from it, and the browser's egress proxy finds the networks it
//! refuses. They share only how a line is read; what each keeps of the
//! routes, and what an unreadable table means (the web fails closed, the
//! proxy takes none), stays with the caller.

use std::net::Ipv4Addr;

/// Where the kernel keeps the table.
pub const PATH: &str = "/proc/net/route";

/// One line of the table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Route<'a> {
    pub iface: &'a str,
    pub destination: Ipv4Addr,
    pub gateway: Ipv4Addr,
    /// `None` when the column is not a number.
    pub metric: Option<u32>,
    pub mask: Ipv4Addr,
}

impl Route<'_> {
    /// The length of the mask's prefix: how many of its bits are set.
    pub fn prefix(&self) -> u8 {
        u32::from(self.mask).count_ones() as u8
    }
}

/// The routes of `table` (the text of `/proc/net/route`): the first line is
/// the header and is skipped whatever it holds, and a line with fewer than
/// eight columns, or whose destination, gateway or mask is not hexadecimal,
/// is skipped.
///
/// Columns: `Iface Destination Gateway Flags RefCnt Use Metric Mask ...`. The
/// kernel writes each address (in network order in memory) as the
/// hexadecimal of a native 32-bit number, so its native bytes are the octets.
/// The flags are not read.
pub fn parse(table: &str) -> impl Iterator<Item = Route<'_>> {
    let address = |hex: &str| {
        u32::from_str_radix(hex, 16)
            .ok()
            .map(|v| Ipv4Addr::from(v.to_ne_bytes()))
    };
    table.lines().skip(1).filter_map(move |line| {
        let fields: Vec<&str> = line.split_whitespace().collect();
        Some(Route {
            iface: fields.first()?,
            destination: address(fields.get(1)?)?,
            gateway: address(fields.get(2)?)?,
            metric: fields.get(6)?.parse().ok(),
            mask: address(fields.get(7)?)?,
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const HEADER: &str =
        "Iface\tDestination\tGateway \tFlags\tRefCnt\tUse\tMetric\tMask\t\tMTU\tWindow\tIRTT\n";

    /// The kernel's text of an address on this machine: the native number
    /// whose native bytes are the octets.
    fn hex(a: u8, b: u8, c: u8, d: u8) -> String {
        format!("{:08X}", u32::from_ne_bytes([a, b, c, d]))
    }

    fn line(iface: &str, destination: &str, gateway: &str, metric: &str, mask: &str) -> String {
        format!("{iface}\t{destination}\t{gateway}\t0003\t0\t0\t{metric}\t{mask}\t0\t0\t0\n")
    }

    #[test]
    fn a_line_is_read_in_the_byte_order_of_the_machine() {
        let table = format!(
            "{HEADER}{}",
            line(
                "eth0",
                &hex(172, 22, 0, 0),
                &hex(172, 22, 0, 1),
                "100",
                &hex(255, 255, 0, 0)
            )
        );
        let routes: Vec<_> = parse(&table).collect();
        assert_eq!(
            routes,
            [Route {
                iface: "eth0",
                destination: Ipv4Addr::new(172, 22, 0, 0),
                gateway: Ipv4Addr::new(172, 22, 0, 1),
                metric: Some(100),
                mask: Ipv4Addr::new(255, 255, 0, 0),
            }]
        );
        assert_eq!(routes[0].prefix(), 16);
        // The all-zero address is the same in any order.
        assert_eq!(
            parse(&format!(
                "{HEADER}{}",
                line("eth0", "00000000", "00000000", "0", "00000000")
            ))
            .next()
            .unwrap()
            .prefix(),
            0
        );
    }

    #[test]
    fn the_first_line_is_skipped_whatever_it_holds() {
        assert_eq!(parse("").count(), 0);
        assert_eq!(parse(HEADER).count(), 0);
        let route = line("eth0", "00000000", "00000000", "0", "00000000");
        assert_eq!(parse(&route).count(), 0, "a route in the header's place");
        assert_eq!(parse(&format!("{HEADER}{route}")).count(), 1);
    }

    #[test]
    fn a_short_line_or_a_bad_address_is_skipped_and_the_rest_are_read() {
        let good = line("eth0", "00000000", "00000000", "0", "00000000");
        let table = format!(
            "{HEADER}\
             eth0\t00000000\t00000000\t0003\t0\t0\t0\n\
             \n\
             garbage\n\
             {}{}{}{good}",
            line("eth0", "zz", "00000000", "0", "00000000"),
            line("eth0", "00000000", "g", "0", "00000000"),
            line("eth0", "00000000", "00000000", "0", "-1"),
        );
        let routes: Vec<_> = parse(&table).collect();
        assert_eq!(routes.len(), 1, "{routes:?}");
        assert_eq!(routes[0].iface, "eth0");
    }

    #[test]
    fn a_metric_that_is_not_a_number_keeps_the_route_without_it() {
        let table = format!(
            "{HEADER}{}{}{}",
            line("eth0", "00000000", "00000000", "7", "00000000"),
            line("eth0", "00000000", "00000000", "x", "00000000"),
            line("eth0", "00000000", "00000000", "-1", "00000000"),
        );
        let metrics: Vec<_> = parse(&table).map(|r| r.metric).collect();
        assert_eq!(metrics, [Some(7), None, None]);
    }

    #[test]
    fn loopback_routes_are_read_like_any_other() {
        let table = format!(
            "{HEADER}{}",
            line(
                "lo",
                &hex(127, 0, 0, 0),
                "00000000",
                "0",
                &hex(255, 0, 0, 0)
            )
        );
        let routes: Vec<_> = parse(&table).collect();
        assert_eq!(routes.len(), 1);
        assert_eq!(routes[0].iface, "lo");
        assert_eq!(routes[0].destination, Ipv4Addr::new(127, 0, 0, 0));
        assert_eq!(routes[0].prefix(), 8);
    }
}
