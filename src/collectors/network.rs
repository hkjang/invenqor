use super::{read_trimmed, record, Collector};
use anyhow::Result;
use serde_json::json;
use std::collections::BTreeMap;
use std::ffi::CStr;
use std::net::{Ipv4Addr, Ipv6Addr};
use std::path::Path;

pub struct NetworkCollector;

impl Collector for NetworkCollector {
    fn name(&self) -> &'static str {
        "network"
    }

    fn collect(&self, collected_at: u64) -> Result<Vec<crate::model::AssetRecord>> {
        let addresses = interface_addresses();
        let mut records = Vec::new();
        for entry in std::fs::read_dir("/sys/class/net")? {
            let entry = entry?;
            let name = entry.file_name().to_string_lossy().into_owned();
            let base = entry.path();
            records.push(record(
                "network.interface",
                "/sys/class/net,getifaddrs",
                collected_at,
                json!({
                    "name": name,
                    "mac": read_trimmed(base.join("address")),
                    "state": read_trimmed(base.join("operstate")),
                    "mtu": read_trimmed(base.join("mtu")).and_then(|v| v.parse::<u64>().ok()),
                    "ifindex": read_trimmed(base.join("ifindex")).and_then(|v| v.parse::<u64>().ok()),
                    "addresses": addresses.get(&name).cloned().unwrap_or_default(),
                }),
            ));
        }

        records.push(record(
            "network.configuration",
            "/proc/net,/etc/resolv.conf",
            collected_at,
            json!({
                "default_routes": default_routes(),
                "dns_servers": dns_servers(),
                "listening": listening_sockets(),
            }),
        ));
        Ok(records)
    }
}

fn interface_addresses() -> BTreeMap<String, Vec<String>> {
    let mut result = BTreeMap::<String, Vec<String>>::new();
    let mut head: *mut libc::ifaddrs = std::ptr::null_mut();
    // SAFETY: getifaddrs initializes head on success; it is released by freeifaddrs below.
    if unsafe { libc::getifaddrs(&mut head) } != 0 {
        return result;
    }
    let mut current = head;
    while !current.is_null() {
        // SAFETY: current traverses the linked list returned by getifaddrs.
        let item = unsafe { &*current };
        if !item.ifa_name.is_null() && !item.ifa_addr.is_null() {
            // SAFETY: ifa_name is a NUL-terminated string for the lifetime of the list.
            let name = unsafe { CStr::from_ptr(item.ifa_name) }
                .to_string_lossy()
                .into_owned();
            // SAFETY: ifa_addr points to a sockaddr whose concrete type is selected by sa_family.
            let address = unsafe {
                match (*item.ifa_addr).sa_family as i32 {
                    libc::AF_INET => {
                        let addr = &*(item.ifa_addr as *const libc::sockaddr_in);
                        Some(Ipv4Addr::from(u32::from_be(addr.sin_addr.s_addr)).to_string())
                    }
                    libc::AF_INET6 => {
                        let addr = &*(item.ifa_addr as *const libc::sockaddr_in6);
                        Some(Ipv6Addr::from(addr.sin6_addr.s6_addr).to_string())
                    }
                    _ => None,
                }
            };
            if let Some(address) = address {
                let values = result.entry(name).or_default();
                if !values.contains(&address) {
                    values.push(address);
                }
            }
        }
        current = item.ifa_next;
    }
    // SAFETY: head was returned by a successful getifaddrs call.
    unsafe { libc::freeifaddrs(head) };
    for values in result.values_mut() {
        values.sort();
    }
    result
}

fn default_routes() -> Vec<serde_json::Value> {
    let text = std::fs::read_to_string("/proc/net/route").unwrap_or_default();
    text.lines()
        .skip(1)
        .filter_map(|line| {
            let fields: Vec<_> = line.split_whitespace().collect();
            if fields.len() < 8 || fields[1] != "00000000" {
                return None;
            }
            let gateway = u32::from_str_radix(fields[2], 16).ok()?;
            let bytes = gateway.to_le_bytes();
            Some(json!({
                "interface": fields[0],
                "gateway": Ipv4Addr::from(bytes).to_string(),
                "metric": fields[6].parse::<u64>().ok(),
            }))
        })
        .collect()
}

fn dns_servers() -> Vec<String> {
    std::fs::read_to_string("/etc/resolv.conf")
        .unwrap_or_default()
        .lines()
        .filter_map(|line| {
            let line = line.split('#').next()?.trim();
            line.strip_prefix("nameserver")
                .map(str::trim)
                .filter(|v| !v.is_empty())
                .map(str::to_string)
        })
        .collect()
}

fn listening_sockets() -> Vec<serde_json::Value> {
    let mut result = Vec::new();
    for (path, protocol, ipv6) in [
        ("/proc/net/tcp", "tcp", false),
        ("/proc/net/tcp6", "tcp6", true),
        ("/proc/net/udp", "udp", false),
        ("/proc/net/udp6", "udp6", true),
    ] {
        if let Ok(text) = std::fs::read_to_string(Path::new(path)) {
            result.extend(parse_sockets(&text, protocol, ipv6));
        }
    }
    result.sort_by_key(|v| {
        (
            v["protocol"].as_str().unwrap_or_default().to_string(),
            v["port"].as_u64().unwrap_or_default(),
        )
    });
    result
}

fn parse_sockets(text: &str, protocol: &str, ipv6: bool) -> Vec<serde_json::Value> {
    text.lines()
        .skip(1)
        .filter_map(|line| {
            let fields: Vec<_> = line.split_whitespace().collect();
            let local = *fields.get(1)?;
            let state = *fields.get(3)?;
            if protocol.starts_with("tcp") && state != "0A" {
                return None;
            }
            let (address, port) = local.split_once(':')?;
            let port = u16::from_str_radix(port, 16).ok()?;
            let address = if ipv6 {
                decode_ipv6(address)?
            } else {
                let raw = u32::from_str_radix(address, 16).ok()?;
                Ipv4Addr::from(raw.to_le_bytes()).to_string()
            };
            Some(json!({"protocol": protocol, "address": address, "port": port}))
        })
        .collect()
}

fn decode_ipv6(value: &str) -> Option<String> {
    if value.len() != 32 {
        return None;
    }
    let mut bytes = [0u8; 16];
    // Linux renders each 32-bit word in host byte order in /proc/net/tcp6.
    for word in 0..4 {
        let raw = u32::from_str_radix(&value[word * 8..word * 8 + 8], 16).ok()?;
        bytes[word * 4..word * 4 + 4].copy_from_slice(&raw.to_le_bytes());
    }
    Some(Ipv6Addr::from(bytes).to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_listening_tcp() {
        let text = " sl local_address rem_address st\n 0: 0100007F:0016 00000000:0000 0A\n";
        let values = parse_sockets(text, "tcp", false);
        assert_eq!(values[0]["address"], "127.0.0.1");
        assert_eq!(values[0]["port"], 22);
    }
}
