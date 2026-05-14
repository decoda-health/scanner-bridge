pub mod escl;
pub mod mock;

use serde::{Deserialize, Serialize};

/// Information about a detected scanner device.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScannerInfo {
    pub id: String,
    pub name: String,
    pub scanner_type: ScannerType,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ScannerType {
    Flatbed,
    Feeder,
    Unknown,
}

/// Options for a scan operation.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScanOptions {
    pub scanner_id: String,
    #[serde(default = "default_dpi")]
    pub dpi: u32,
    #[serde(default)]
    pub color_mode: ColorMode,
    #[serde(default)]
    pub format: OutputFormat,
}

fn default_dpi() -> u32 {
    300
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ColorMode {
    #[default]
    Color,
    Grayscale,
    BlackWhite,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OutputFormat {
    #[default]
    Pdf,
    Png,
    Jpeg,
}

/// A single scanned page as raw image bytes.
#[derive(Debug, Clone)]
pub struct ScannedPage {
    pub png_data: Vec<u8>,
    pub width: u32,
    pub height: u32,
}

/// The platform-agnostic scanner interface.
/// Each platform (macOS, Windows) implements this, plus a mock for testing.
pub trait ScannerBackend: Send + Sync {
    /// List all available scanners.
    fn list_scanners(&self) -> Vec<ScannerInfo>;

    /// Perform a scan and return the resulting pages.
    /// The callback is invoked for progress updates (page number).
    fn scan(
        &self,
        options: &ScanOptions,
        on_progress: Box<dyn Fn(usize) + Send>,
    ) -> Result<Vec<ScannedPage>, ScanError>;
}

#[derive(Debug, Clone, Serialize)]
pub struct ScanError {
    pub message: String,
}

impl std::fmt::Display for ScanError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for ScanError {}

impl From<String> for ScanError {
    fn from(message: String) -> Self {
        ScanError { message }
    }
}

impl From<&str> for ScanError {
    fn from(message: &str) -> Self {
        ScanError {
            message: message.to_string(),
        }
    }
}

/// Create the appropriate scanner backend for the current platform.
/// Use `--mock` flag or `mock-scanner` feature to use the mock backend.
pub fn create_backend(use_mock: bool) -> Box<dyn ScannerBackend> {
    if use_mock || cfg!(feature = "mock-scanner") {
        tracing::info!("Using mock scanner backend");
        return Box::new(mock::MockScanner::new());
    }

    tracing::info!("Using eSCL (AirScan) scanner backend");
    Box::new(escl::EsclScanner::new())
}
