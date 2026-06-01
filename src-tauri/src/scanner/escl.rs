//! eSCL (AirScan) scanner discovery and scanning — cross-platform.
//!
//! Discovery uses platform-native mDNS tools to find scanners advertising
//! `_uscan._tcp` (eSCL) on the local network. Scanning is done via HTTP.

use std::collections::HashMap;
use std::process::Command;
use std::sync::{Mutex, OnceLock};

use super::{ScanError, ScanOptions, ScannedPage, ScannerBackend, ScannerInfo, ScannerType};

/// Cache of discovered scanners: UUID → (name, host)
static ESCL_SCANNERS: OnceLock<Mutex<HashMap<String, (String, String)>>> = OnceLock::new();

fn escl_scanners() -> &'static Mutex<HashMap<String, (String, String)>> {
    ESCL_SCANNERS.get_or_init(|| Mutex::new(HashMap::new()))
}

// ---------------------------------------------------------------------------
// Discovery via platform-native mDNS
// ---------------------------------------------------------------------------

const DNS_SD_TIMEOUT_SECS: u64 = 10;

/// Run a command with a timeout (dns-sd never exits on its own).
fn run_with_timeout(
    cmd: &str,
    args: &[&str],
    timeout_secs: u64,
) -> std::io::Result<std::process::Output> {
    let mut child = Command::new(cmd)
        .args(args)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()?;

    std::thread::sleep(std::time::Duration::from_secs(timeout_secs));
    let _ = child.kill();
    child.wait_with_output()
}

/// Discover eSCL scanners on the local network.
/// Uses `dns-sd` (macOS) or `dns-sd.exe` (Windows with Bonjour installed).
fn discover_scanners() {
    let output = run_with_timeout(
        "dns-sd",
        &["-B", "_uscan._tcp", "local"],
        DNS_SD_TIMEOUT_SECS,
    );

    let output = match output {
        Ok(o) => o,
        Err(e) => {
            tracing::warn!("Scanner discovery failed (dns-sd not available?): {e}");
            return;
        }
    };

    let text = combined_output(&output);

    // Parse instance names from dns-sd -B output
    let mut names: Vec<String> = Vec::new();
    for line in text.lines() {
        if !line.contains("_uscan._tcp.")
            || line.contains("STARTING")
            || line.starts_with("Browsing for ")
            || line.starts_with("Timestamp")
        {
            continue;
        }
        if let Some(idx) = line.find("_uscan._tcp.") {
            let after = &line[idx + "_uscan._tcp.".len()..];
            let name = after.trim().to_string();
            if !name.is_empty() && !names.contains(&name) {
                names.push(name);
            }
        }
    }

    for name in &names {
        resolve_scanner(name);
    }
}

/// Resolve a scanner's hostname and UUID.
fn resolve_scanner(instance_name: &str) {
    let output = run_with_timeout(
        "dns-sd",
        &["-L", instance_name, "_uscan._tcp", "local"],
        DNS_SD_TIMEOUT_SECS,
    );

    let output = match output {
        Ok(o) => o,
        Err(e) => {
            tracing::warn!("dns-sd lookup failed for {instance_name}: {e}");
            return;
        }
    };

    let text = combined_output(&output);
    let mut hostname: Option<String> = None;
    let mut uuid: Option<String> = None;

    for line in text.lines() {
        hostname = hostname.or_else(|| parse_reachable_host(line));
        uuid = uuid.or_else(|| parse_uuid(line));
    }

    if let Some(host) = hostname {
        let id = uuid.unwrap_or_else(|| fallback_scanner_id(instance_name, &host));
        tracing::info!("Discovered eSCL scanner: {instance_name} -> {host} (id={id})");
        if let Ok(mut map) = escl_scanners().lock() {
            map.insert(id, (instance_name.to_string(), host));
        }
    } else {
        tracing::warn!("Could not resolve eSCL scanner '{instance_name}'. dns-sd output: {text}");
    }
}

fn combined_output(output: &std::process::Output) -> String {
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    format!("{stdout}{stderr}")
}

fn parse_reachable_host(line: &str) -> Option<String> {
    let after = line.split_once("can be reached at ")?.1;
    let mut parts = after.split_whitespace();
    let raw_host = parts.next()?.trim_end_matches('.');
    let mut host = raw_host.to_string();

    if let Some((name, port)) = raw_host.rsplit_once(':') {
        host = format!("{}:{}", name.trim_end_matches('.'), port);
    } else {
        let tokens: Vec<&str> = after.split_whitespace().collect();
        for window in tokens.windows(2) {
            if window[0].eq_ignore_ascii_case("port") {
                host = format!("{}:{}", raw_host.trim_end_matches('.'), window[1]);
                break;
            }
        }
    }

    (!host.is_empty()).then_some(host)
}

fn parse_uuid(line: &str) -> Option<String> {
    let lower = line.to_ascii_lowercase();
    let idx = lower.find("uuid=")?;
    let after = &line[idx + "uuid=".len()..];
    let id = after
        .trim_start_matches(|c| c == '"' || c == '\'')
        .split(|c: char| c.is_whitespace() || c == '"' || c == '\'')
        .next()?
        .trim_matches(|c| c == '"' || c == '\'' || c == ';')
        .to_string();

    (!id.is_empty()).then_some(id)
}

fn fallback_scanner_id(instance_name: &str, host: &str) -> String {
    format!("escl:{host}:{instance_name}")
}

#[cfg(test)]
mod tests {
    use super::{fallback_scanner_id, parse_reachable_host, parse_uuid};

    #[test]
    fn parses_dns_sd_reachable_host_with_colon_port() {
        let line = "12:40:48.111 Brother MFC._uscan._tcp.local. can be reached at BRWABC123.local.:80 (interface 14)";

        assert_eq!(
            parse_reachable_host(line).as_deref(),
            Some("BRWABC123.local:80")
        );
    }

    #[test]
    fn parses_dns_sd_reachable_host_with_port_token() {
        let line = "Brother MFC._uscan._tcp.local. can be reached at BRWABC123.local. port 8080";

        assert_eq!(
            parse_reachable_host(line).as_deref(),
            Some("BRWABC123.local:8080")
        );
    }

    #[test]
    fn parses_uuid_case_insensitively() {
        assert_eq!(
            parse_uuid(r#"txtvers=1 UUID=E3248000-80CE-11DB-8000-30055CABCDEF"#).as_deref(),
            Some("E3248000-80CE-11DB-8000-30055CABCDEF")
        );
        assert_eq!(
            parse_uuid(r#""txtvers=1" "uuid=e3248000-80ce-11db-8000-30055cabcdef""#).as_deref(),
            Some("e3248000-80ce-11db-8000-30055cabcdef")
        );
    }

    #[test]
    fn builds_fallback_id_when_uuid_is_absent() {
        assert_eq!(
            fallback_scanner_id("Brother MFC-L8900CDW series", "BRWABC123.local:80"),
            "escl:BRWABC123.local:80:Brother MFC-L8900CDW series"
        );
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
        // Run initial discovery
        discover_scanners();

        // Periodically re-discover
        std::thread::spawn(|| loop {
            std::thread::sleep(std::time::Duration::from_secs(15));
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
