//! eSCL (AirScan) scanner discovery and scanning — cross-platform.
//!
//! Discovery uses a bundled Rust mDNS/DNS-SD querier to find scanners advertising
//! `_uscan._tcp` (eSCL) on the local network. Scanning is done via HTTP.

use std::cmp::Ordering;
use std::collections::HashMap;
use std::io;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddrV4, UdpSocket};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use if_addrs::{get_if_addrs, IfAddr};

use super::{ScanError, ScanOptions, ScannedPage, ScannerBackend, ScannerInfo, ScannerType};

/// Cache of discovered scanners: UUID → (name, host)
static ESCL_SCANNERS: OnceLock<Mutex<HashMap<String, (String, String)>>> = OnceLock::new();

fn escl_scanners() -> &'static Mutex<HashMap<String, (String, String)>> {
    ESCL_SCANNERS.get_or_init(|| Mutex::new(HashMap::new()))
}

// ---------------------------------------------------------------------------
// Discovery via bundled Rust mDNS/DNS-SD
// ---------------------------------------------------------------------------

const ESCL_SERVICE_TYPE: &str = "_uscan._tcp.local.";
const MDNS_GROUP_V4: Ipv4Addr = Ipv4Addr::new(224, 0, 0, 251);
const MDNS_PORT: u16 = 5353;
const DNS_TYPE_A: u16 = 1;
const DNS_TYPE_PTR: u16 = 12;
const DNS_TYPE_TXT: u16 = 16;
const DNS_TYPE_AAAA: u16 = 28;
const DNS_TYPE_SRV: u16 = 33;
const DNS_CLASS_IN: u16 = 1;
const LEGACY_MDNS_QUERY_ID: u16 = 0xdec0;
const DISCOVERY_TIMEOUT: Duration = Duration::from_secs(10);
const REDISCOVERY_INTERVAL: Duration = Duration::from_secs(15);
const QUERY_INTERVAL: Duration = Duration::from_millis(500);
const SOCKET_READ_TIMEOUT: Duration = Duration::from_millis(100);

/// Discover eSCL scanners on the local network.
fn discover_scanners() {
    let scanners = discover_scanners_with_rust_mdns();

    if scanners.is_empty() {
        tracing::warn!("No eSCL scanners resolved during discovery window");
        return;
    }

    if let Ok(mut map) = escl_scanners().lock() {
        for scanner in scanners {
            tracing::info!(
                "Discovered eSCL scanner: {} -> {} (id={})",
                scanner.name,
                scanner.host,
                scanner.id
            );
            map.insert(scanner.id, (scanner.name, scanner.host));
        }
    }
}

fn discover_scanners_with_rust_mdns() -> Vec<ResolvedScanner> {
    let sockets = discovery_sockets();
    if sockets.is_empty() {
        tracing::warn!("No IPv4 network interface available for eSCL mDNS discovery");
        return Vec::new();
    }

    let query = build_mdns_query(ESCL_SERVICE_TYPE, DNS_TYPE_PTR);
    let destination = SocketAddrV4::new(MDNS_GROUP_V4, MDNS_PORT);
    let deadline = Instant::now() + DISCOVERY_TIMEOUT;
    let mut next_query = Instant::now();
    let mut discovery = MdnsDiscovery::default();
    let mut buf = [0_u8; 4096];

    while Instant::now() < deadline {
        let now = Instant::now();
        if now >= next_query {
            for (interface, socket) in &sockets {
                if let Err(e) = socket.send_to(&query, destination) {
                    tracing::debug!("Could not send eSCL mDNS query on {interface}: {e}");
                }
            }
            next_query = now + QUERY_INTERVAL;
        }

        for (interface, socket) in &sockets {
            match socket.recv_from(&mut buf) {
                Ok((len, source)) => {
                    tracing::debug!(
                        "Received eSCL mDNS response from {source} on {interface} ({len} bytes)"
                    );
                    if let Err(e) = discovery.add_packet(&buf[..len]) {
                        tracing::debug!("Ignoring malformed eSCL mDNS response from {source}: {e}");
                    }
                }
                Err(e)
                    if matches!(
                        e.kind(),
                        io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                    ) => {}
                Err(e) => {
                    tracing::debug!("Could not receive eSCL mDNS response on {interface}: {e}");
                }
            }
        }
    }

    discovery.resolved_scanners()
}

fn discovery_sockets() -> Vec<(String, UdpSocket)> {
    let mut sockets = Vec::new();

    match get_if_addrs() {
        Ok(interfaces) => {
            for interface in interfaces {
                if interface.is_loopback() || interface.is_p2p() || !interface.is_oper_up() {
                    continue;
                }

                let IfAddr::V4(addr) = interface.addr else {
                    continue;
                };
                let label = format!("{} ({})", interface.name, addr.ip);

                match UdpSocket::bind(SocketAddrV4::new(addr.ip, 0)) {
                    Ok(socket) => {
                        configure_discovery_socket(&socket, &label);
                        sockets.push((label, socket));
                    }
                    Err(e) => {
                        tracing::debug!("Could not bind eSCL mDNS socket on {label}: {e}");
                    }
                }
            }
        }
        Err(e) => {
            tracing::warn!("Could not enumerate network interfaces for eSCL mDNS discovery: {e}");
        }
    }

    if sockets.is_empty() {
        match UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0)) {
            Ok(socket) => {
                let label = Ipv4Addr::UNSPECIFIED.to_string();
                configure_discovery_socket(&socket, &label);
                sockets.push((label, socket));
            }
            Err(e) => {
                tracing::debug!("Could not bind wildcard eSCL mDNS discovery socket: {e}");
            }
        }
    }

    sockets
}

fn configure_discovery_socket(socket: &UdpSocket, label: &str) {
    if let Err(e) = socket.set_read_timeout(Some(SOCKET_READ_TIMEOUT)) {
        tracing::debug!("Could not set eSCL mDNS read timeout on {label}: {e}");
    }
    if let Err(e) = socket.set_multicast_ttl_v4(255) {
        tracing::debug!("Could not set eSCL mDNS multicast TTL on {label}: {e}");
    }
}

#[derive(Debug, Default)]
struct MdnsDiscovery {
    services: HashMap<String, MdnsService>,
    addresses: HashMap<String, Vec<IpAddr>>,
}

impl MdnsDiscovery {
    fn add_packet(&mut self, packet: &[u8]) -> Result<(), String> {
        if packet.len() < 12 {
            return Err("DNS packet header is too short".to_string());
        }

        let question_count = read_u16(packet, 4).unwrap_or(0) as usize;
        let answer_count = read_u16(packet, 6).unwrap_or(0) as usize;
        let authority_count = read_u16(packet, 8).unwrap_or(0) as usize;
        let additional_count = read_u16(packet, 10).unwrap_or(0) as usize;

        let mut offset = 12;
        for _ in 0..question_count {
            let (_, next_offset) = read_dns_name(packet, offset)?;
            offset = next_offset
                .checked_add(4)
                .ok_or_else(|| "DNS question offset overflow".to_string())?;
            if offset > packet.len() {
                return Err("DNS question extends past packet".to_string());
            }
        }

        let record_count = answer_count + authority_count + additional_count;
        for _ in 0..record_count {
            let (name, next_offset) = read_dns_name(packet, offset)?;
            offset = next_offset;

            if offset + 10 > packet.len() {
                return Err("DNS record header extends past packet".to_string());
            }

            let record_type = read_u16(packet, offset).unwrap_or(0);
            let record_class = read_u16(packet, offset + 2).unwrap_or(0) & 0x7fff;
            let data_len = read_u16(packet, offset + 8).unwrap_or(0) as usize;
            let data_offset = offset + 10;
            let next_record = data_offset
                .checked_add(data_len)
                .ok_or_else(|| "DNS record offset overflow".to_string())?;
            if next_record > packet.len() {
                return Err("DNS record data extends past packet".to_string());
            }

            if record_class == DNS_CLASS_IN {
                match record_type {
                    DNS_TYPE_PTR => {
                        let (target, _) = read_dns_name(packet, data_offset)?;
                        if dns_key(&name) == dns_key(ESCL_SERVICE_TYPE)
                            || dns_key(&target).ends_with(&dns_key(ESCL_SERVICE_TYPE))
                        {
                            self.ensure_service(&target);
                        }
                    }
                    DNS_TYPE_SRV => {
                        if data_len >= 6 {
                            let port = read_u16(packet, data_offset + 4).unwrap_or(0);
                            let (target, _) = read_dns_name(packet, data_offset + 6)?;
                            let service = self.ensure_service(&name);
                            service.target = Some(target);
                            service.port = Some(port);
                        }
                    }
                    DNS_TYPE_TXT => {
                        let txt = parse_txt_properties(&packet[data_offset..next_record]);
                        if !txt.is_empty() {
                            let service = self.ensure_service(&name);
                            service.txt.extend(txt);
                        }
                    }
                    DNS_TYPE_A if data_len == 4 => {
                        let ip = IpAddr::V4(Ipv4Addr::new(
                            packet[data_offset],
                            packet[data_offset + 1],
                            packet[data_offset + 2],
                            packet[data_offset + 3],
                        ));
                        self.add_address(&name, ip);
                    }
                    DNS_TYPE_AAAA if data_len == 16 => {
                        let mut octets = [0_u8; 16];
                        octets.copy_from_slice(&packet[data_offset..next_record]);
                        self.add_address(&name, IpAddr::V6(Ipv6Addr::from(octets)));
                    }
                    _ => {}
                }
            }

            offset = next_record;
        }

        Ok(())
    }

    fn ensure_service(&mut self, fullname: &str) -> &mut MdnsService {
        let key = dns_key(fullname);
        self.services
            .entry(key)
            .or_insert_with(|| MdnsService::new(fullname))
    }

    fn add_address(&mut self, hostname: &str, ip: IpAddr) {
        let addresses = self.addresses.entry(dns_key(hostname)).or_default();
        if !addresses.contains(&ip) {
            addresses.push(ip);
        }
    }

    fn resolved_scanners(&self) -> Vec<ResolvedScanner> {
        let mut scanners = Vec::new();

        for service in self.services.values() {
            let Some(port) = service.port else {
                continue;
            };
            let Some(target) = service.target.as_deref() else {
                continue;
            };

            let addresses = self.addresses.get(&dns_key(target));
            let host = addresses
                .and_then(|items| preferred_ip(items.iter().copied()))
                .map(|ip| format_ip_endpoint(ip, port))
                .or_else(|| format_hostname_endpoint(target, port));

            let Some(host) = host else {
                continue;
            };

            let id = service
                .txt
                .get("uuid")
                .map(|uuid| uuid.trim())
                .filter(|uuid| !uuid.is_empty())
                .map(ToOwned::to_owned)
                .unwrap_or_else(|| fallback_scanner_id(&service.name, &host));

            scanners.push(ResolvedScanner {
                id,
                name: service.name.clone(),
                host,
            });
        }

        scanners.sort_by(|left, right| left.name.cmp(&right.name));
        scanners
    }
}

#[derive(Debug)]
struct MdnsService {
    name: String,
    target: Option<String>,
    port: Option<u16>,
    txt: HashMap<String, String>,
}

impl MdnsService {
    fn new(fullname: &str) -> Self {
        Self {
            name: service_instance_name(fullname),
            target: None,
            port: None,
            txt: HashMap::new(),
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
struct ResolvedScanner {
    id: String,
    name: String,
    host: String,
}

fn build_mdns_query(name: &str, query_type: u16) -> Vec<u8> {
    let mut packet = Vec::with_capacity(32 + name.len());
    packet.extend_from_slice(&LEGACY_MDNS_QUERY_ID.to_be_bytes());
    packet.extend_from_slice(&0_u16.to_be_bytes());
    packet.extend_from_slice(&1_u16.to_be_bytes());
    packet.extend_from_slice(&0_u16.to_be_bytes());
    packet.extend_from_slice(&0_u16.to_be_bytes());
    packet.extend_from_slice(&0_u16.to_be_bytes());
    write_dns_name(name, &mut packet);
    packet.extend_from_slice(&query_type.to_be_bytes());
    packet.extend_from_slice(&DNS_CLASS_IN.to_be_bytes());
    packet
}

fn write_dns_name(name: &str, packet: &mut Vec<u8>) {
    for label in name.trim_end_matches('.').split('.') {
        packet.push(label.len() as u8);
        packet.extend_from_slice(label.as_bytes());
    }
    packet.push(0);
}

fn read_u16(packet: &[u8], offset: usize) -> Option<u16> {
    Some(u16::from_be_bytes([
        *packet.get(offset)?,
        *packet.get(offset + 1)?,
    ]))
}

fn read_dns_name(packet: &[u8], offset: usize) -> Result<(String, usize), String> {
    let mut labels = Vec::new();
    let mut pos = offset;
    let mut next_offset = offset;
    let mut jumped = false;
    let mut jump_count = 0;

    loop {
        let Some(&len) = packet.get(pos) else {
            return Err("DNS name extends past packet".to_string());
        };

        if len & 0xc0 == 0xc0 {
            let Some(&next) = packet.get(pos + 1) else {
                return Err("DNS name pointer extends past packet".to_string());
            };
            if !jumped {
                next_offset = pos + 2;
            }
            pos = (((len & 0x3f) as usize) << 8) | next as usize;
            jumped = true;
            jump_count += 1;
            if jump_count > packet.len() {
                return Err("DNS name contains a pointer loop".to_string());
            }
            continue;
        }

        if len & 0xc0 != 0 {
            return Err("DNS name uses an unsupported label encoding".to_string());
        }

        pos += 1;
        if len == 0 {
            if !jumped {
                next_offset = pos;
            }
            break;
        }

        let end = pos + len as usize;
        if end > packet.len() {
            return Err("DNS label extends past packet".to_string());
        }

        labels.push(String::from_utf8_lossy(&packet[pos..end]).into_owned());
        pos = end;
    }

    Ok((format!("{}.", labels.join(".")), next_offset))
}

fn parse_txt_properties(data: &[u8]) -> HashMap<String, String> {
    let mut properties = HashMap::new();
    let mut offset = 0;

    while offset < data.len() {
        let len = data[offset] as usize;
        offset += 1;
        if offset + len > data.len() {
            break;
        }

        let item = String::from_utf8_lossy(&data[offset..offset + len]);
        if let Some((key, value)) = item.split_once('=') {
            properties.insert(key.to_ascii_lowercase(), value.to_string());
        } else if !item.is_empty() {
            properties.insert(item.to_ascii_lowercase(), String::new());
        }
        offset += len;
    }

    properties
}

fn preferred_ip(addresses: impl IntoIterator<Item = IpAddr>) -> Option<IpAddr> {
    let mut ips: Vec<IpAddr> = addresses
        .into_iter()
        .filter(|ip| !ip.is_loopback())
        .collect();

    ips.sort_by(|left, right| match (left, right) {
        (IpAddr::V4(_), IpAddr::V6(_)) => Ordering::Less,
        (IpAddr::V6(_), IpAddr::V4(_)) => Ordering::Greater,
        _ => left.to_string().cmp(&right.to_string()),
    });

    ips.into_iter().next()
}

fn format_ip_endpoint(ip: IpAddr, port: u16) -> String {
    match ip {
        IpAddr::V4(addr) => format!("{addr}:{port}"),
        IpAddr::V6(addr) => format!("[{addr}]:{port}"),
    }
}

fn format_hostname_endpoint(hostname: &str, port: u16) -> Option<String> {
    let host = hostname.trim().trim_end_matches('.');
    if host.is_empty() {
        return None;
    }

    let host = if host.contains(':') && !host.starts_with('[') {
        format!("[{host}]")
    } else {
        host.to_string()
    };

    Some(format!("{host}:{port}"))
}

fn service_instance_name(fullname: &str) -> String {
    let trimmed = fullname.trim_end_matches('.');
    let suffix = format!(".{}", ESCL_SERVICE_TYPE.trim_end_matches('.'));

    if trimmed.to_ascii_lowercase().ends_with(&suffix) {
        trimmed[..trimmed.len() - suffix.len()].to_string()
    } else {
        trimmed.to_string()
    }
}

fn dns_key(name: &str) -> String {
    name.trim_end_matches('.').to_ascii_lowercase()
}

fn fallback_scanner_id(instance_name: &str, host: &str) -> String {
    format!("escl:{host}:{instance_name}")
}

#[cfg(test)]
mod tests {
    use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

    use super::{
        fallback_scanner_id, format_hostname_endpoint, format_ip_endpoint, parse_txt_properties,
        read_dns_name, service_instance_name, write_dns_name, LEGACY_MDNS_QUERY_ID,
    };

    #[test]
    fn extracts_instance_name_from_full_service_name() {
        assert_eq!(
            service_instance_name("Brother MFC-L8900CDW series._uscan._tcp.local."),
            "Brother MFC-L8900CDW series"
        );
    }

    #[test]
    fn leaves_plain_instance_name_unchanged() {
        assert_eq!(
            service_instance_name("Canon MF750C II Series"),
            "Canon MF750C II Series"
        );
    }

    #[test]
    fn builds_fallback_id_when_uuid_is_absent() {
        assert_eq!(
            fallback_scanner_id("Brother MFC-L8900CDW series", "BRWABC123.local:80"),
            "escl:BRWABC123.local:80:Brother MFC-L8900CDW series"
        );
    }

    #[test]
    fn formats_ipv4_endpoint_with_port() {
        assert_eq!(
            format_ip_endpoint(IpAddr::V4(Ipv4Addr::new(192, 168, 1, 42)), 80),
            "192.168.1.42:80"
        );
    }

    #[test]
    fn brackets_ipv6_endpoint_with_port() {
        assert_eq!(
            format_ip_endpoint(IpAddr::V6(Ipv6Addr::LOCALHOST), 8080),
            "[::1]:8080"
        );
    }

    #[test]
    fn formats_hostname_endpoint_with_port() {
        assert_eq!(
            format_hostname_endpoint("BRWABC123.local.", 80).as_deref(),
            Some("BRWABC123.local:80")
        );
    }

    #[test]
    fn parses_txt_properties_with_case_insensitive_keys() {
        let mut txt = Vec::new();
        txt.push("UUID=6d4ff0ce".len() as u8);
        txt.extend_from_slice(b"UUID=6d4ff0ce");
        txt.push("rs=eSCL".len() as u8);
        txt.extend_from_slice(b"rs=eSCL");

        let properties = parse_txt_properties(&txt);

        assert_eq!(properties.get("uuid").map(String::as_str), Some("6d4ff0ce"));
        assert_eq!(properties.get("rs").map(String::as_str), Some("eSCL"));
    }

    #[test]
    fn reads_compressed_dns_name() {
        let mut packet = vec![0; 12];
        write_dns_name("Canon MF750C II Series._uscan._tcp.local.", &mut packet);
        let pointer_offset = packet.len();
        packet.extend_from_slice(&[0xc0, 0x0c]);

        let (name, next_offset) = read_dns_name(&packet, pointer_offset).unwrap();

        assert_eq!(name, "Canon MF750C II Series._uscan._tcp.local.");
        assert_eq!(next_offset, pointer_offset + 2);
    }

    #[test]
    fn builds_legacy_unicast_mdns_query() {
        let query = super::build_mdns_query("_uscan._tcp.local.", super::DNS_TYPE_PTR);

        assert_eq!(&query[0..2], &LEGACY_MDNS_QUERY_ID.to_be_bytes());
        assert_eq!(&query[4..6], &1_u16.to_be_bytes());
    }
}

// ---------------------------------------------------------------------------
// eSCL HTTP scanning
// ---------------------------------------------------------------------------

pub fn escl_scan(
    host: &str,
    options: &ScanOptions,
    on_progress: &dyn Fn(usize),
) -> Result<Vec<ScannedPage>, ScanError> {
    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(120))
        .build()
        .map_err(|e| ScanError::from(format!("HTTP client error: {e}")))?;

    let base_url = format!("http://{host}/eSCL");

    // Verify the endpoint is reachable
    let caps = client
        .get(format!("{base_url}/ScannerCapabilities"))
        .send()
        .map_err(|e| ScanError::from(format!("Cannot reach scanner at {host}: {e}")))?;

    if !caps.status().is_success() {
        return Err(ScanError::from(format!(
            "Scanner at {host} returned HTTP {}",
            caps.status()
        )));
    }

    let color_mode = match options.color_mode {
        super::ColorMode::Color => "RGB24",
        super::ColorMode::Grayscale => "Grayscale8",
        super::ColorMode::BlackWhite => "Grayscale8",
    };

    let scan_settings = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<scan:ScanSettings xmlns:scan="http://schemas.hp.com/imaging/escl/2011/05/03"
                   xmlns:pwg="http://www.pwg.org/schemas/2010/12/sm">
    <pwg:Version>2.0</pwg:Version>
    <scan:Intent>Document</scan:Intent>
    <pwg:ScanRegions>
        <pwg:ScanRegion>
            <pwg:ContentRegionUnits>escl:ThreeHundredthsOfInches</pwg:ContentRegionUnits>
            <pwg:XOffset>0</pwg:XOffset>
            <pwg:YOffset>0</pwg:YOffset>
            <pwg:Width>2550</pwg:Width>
            <pwg:Height>3300</pwg:Height>
        </pwg:ScanRegion>
    </pwg:ScanRegions>
    <pwg:InputSource>Platen</pwg:InputSource>
    <scan:ColorMode>{color_mode}</scan:ColorMode>
    <scan:XResolution>{dpi}</scan:XResolution>
    <scan:YResolution>{dpi}</scan:YResolution>
    <pwg:DocumentFormat>image/jpeg</pwg:DocumentFormat>
</scan:ScanSettings>"#,
        dpi = options.dpi,
    );

    on_progress(0);

    tracing::info!("Creating eSCL scan job on {host}...");
    let resp = client
        .post(format!("{base_url}/ScanJobs"))
        .header("Content-Type", "text/xml")
        .body(scan_settings)
        .send()
        .map_err(|e| ScanError::from(format!("Failed to create scan job: {e}")))?;

    if resp.status().as_u16() != 201 {
        let status = resp.status();
        let body = resp.text().unwrap_or_default();
        return Err(ScanError::from(format!(
            "Scanner rejected scan job: HTTP {status}: {body}"
        )));
    }

    let job_url = resp
        .headers()
        .get("location")
        .and_then(|v| v.to_str().ok())
        .map(|loc| {
            if loc.starts_with("http") {
                loc.to_string()
            } else {
                format!("http://{host}{loc}")
            }
        })
        .ok_or_else(|| ScanError::from("No Location header in scan job response"))?;

    tracing::info!("Scan job created: {job_url}");
    tracing::info!("Waiting for scan to complete...");

    let img_resp = client
        .get(format!("{job_url}/NextDocument"))
        .send()
        .map_err(|e| ScanError::from(format!("Failed to retrieve scan: {e}")))?;

    if !img_resp.status().is_success() {
        return Err(ScanError::from(format!(
            "Scanner returned HTTP {} when fetching image",
            img_resp.status()
        )));
    }

    let image_data = img_resp
        .bytes()
        .map_err(|e| ScanError::from(format!("Failed to read scan data: {e}")))?;

    tracing::info!("Received {} bytes of image data", image_data.len());
    on_progress(1);

    let img = image::load_from_memory(&image_data)
        .map_err(|e| ScanError::from(format!("Failed to decode scanned image: {e}")))?;

    let width = img.width();
    let height = img.height();

    let mut png_data = Vec::new();
    img.write_to(
        &mut std::io::Cursor::new(&mut png_data),
        image::ImageFormat::Png,
    )
    .map_err(|e| ScanError::from(format!("Failed to encode PNG: {e}")))?;

    tracing::info!(
        "Scan complete: {width}x{height} PNG ({} bytes)",
        png_data.len()
    );

    Ok(vec![ScannedPage {
        png_data,
        width,
        height,
    }])
}

// ---------------------------------------------------------------------------
// EsclScanner backend
// ---------------------------------------------------------------------------

pub struct EsclScanner;

impl Default for EsclScanner {
    fn default() -> Self {
        Self::new()
    }
}

impl EsclScanner {
    pub fn new() -> Self {
        discover_scanners();

        std::thread::spawn(|| loop {
            std::thread::sleep(REDISCOVERY_INTERVAL);
            discover_scanners();
        });

        EsclScanner
    }
}

impl ScannerBackend for EsclScanner {
    fn list_scanners(&self) -> Vec<ScannerInfo> {
        let map = escl_scanners().lock().unwrap_or_else(|e| e.into_inner());
        map.iter()
            .map(|(id, (name, _host))| ScannerInfo {
                id: id.clone(),
                name: name.clone(),
                scanner_type: ScannerType::Flatbed,
            })
            .collect()
    }

    fn scan(
        &self,
        options: &ScanOptions,
        on_progress: Box<dyn Fn(usize) + Send>,
    ) -> Result<Vec<ScannedPage>, ScanError> {
        let host = {
            let map = escl_scanners()
                .lock()
                .map_err(|e| ScanError::from(e.to_string()))?;
            map.get(&options.scanner_id)
                .map(|(_name, host)| host.clone())
        };

        let host = host.ok_or_else(|| {
            ScanError::from(format!(
                "Scanner '{}' not found. Try refreshing.",
                options.scanner_id
            ))
        })?;

        tracing::info!("Starting eSCL scan on {host}");
        escl_scan(&host, options, &*on_progress)
    }
}
