use scanner_bridge::pdf::pages_to_pdf;
use scanner_bridge::scanner::mock::MockScanner;
use scanner_bridge::scanner::{ScanOptions, ScannedPage, ScannerBackend};

fn scan_pages(scanner_id: &str, dpi: u32) -> Vec<ScannedPage> {
    let backend = MockScanner::new();
    let options = ScanOptions {
        scanner_id: scanner_id.to_string(),
        dpi,
        color_mode: Default::default(),
        format: Default::default(),
    };
    backend
        .scan(&options, Box::new(|_| {}))
        .expect("Scan should succeed")
}

#[test]
fn single_page_pdf() {
    let pages = scan_pages("mock-flatbed-001", 72);
    let pdf_bytes = pages_to_pdf(&pages, 72).expect("PDF generation should succeed");

    // PDF should start with %PDF
    assert!(pdf_bytes.len() > 100);
    assert_eq!(&pdf_bytes[0..5], b"%PDF-");
}

#[test]
fn multi_page_pdf() {
    let pages = scan_pages("mock-feeder-001", 72);
    assert_eq!(pages.len(), 3);

    let pdf_bytes = pages_to_pdf(&pages, 72).expect("PDF generation should succeed");

    assert_eq!(&pdf_bytes[0..5], b"%PDF-");
    // Multi-page PDF should be larger than single page
    let single_page_pdf = pages_to_pdf(&pages[..1], 72).expect("Single page PDF should work");
    assert!(pdf_bytes.len() > single_page_pdf.len());
}

#[test]
fn empty_pages_returns_error() {
    let result = pages_to_pdf(&[], 300);
    assert!(result.is_err());
    assert_eq!(result.unwrap_err(), "No pages to convert");
}
