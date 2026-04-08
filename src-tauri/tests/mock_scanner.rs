use scanner_bridge::scanner::mock::MockScanner;
use scanner_bridge::scanner::{ScanOptions, ScannerBackend, ScannerType};

#[test]
fn list_scanners_returns_two_devices() {
    let backend = MockScanner::new();
    let scanners = backend.list_scanners();

    assert_eq!(scanners.len(), 2);
    assert_eq!(scanners[0].id, "mock-flatbed-001");
    assert_eq!(scanners[1].id, "mock-feeder-001");
    assert!(matches!(scanners[0].scanner_type, ScannerType::Flatbed));
    assert!(matches!(scanners[1].scanner_type, ScannerType::Feeder));
}

#[test]
fn flatbed_scan_returns_one_page() {
    let backend = MockScanner::new();
    let options = ScanOptions {
        scanner_id: "mock-flatbed-001".to_string(),
        dpi: 150, // Low DPI for faster test
        color_mode: Default::default(),
        format: Default::default(),
    };

    let progress_pages_clone = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let progress_ref = progress_pages_clone.clone();

    let result = backend.scan(
        &options,
        Box::new(move |page| {
            progress_ref.lock().unwrap().push(page);
        }),
    );

    let progress_pages = progress_pages_clone.lock().unwrap().clone();

    let pages = result.expect("Scan should succeed");
    assert_eq!(pages.len(), 1);
    assert_eq!(progress_pages, vec![1]);

    // Verify the page contains valid PNG data
    let page = &pages[0];
    assert!(!page.png_data.is_empty());
    assert!(page.width > 0);
    assert!(page.height > 0);
    // PNG magic bytes
    assert_eq!(&page.png_data[0..4], &[0x89, 0x50, 0x4E, 0x47]);
}

#[test]
fn feeder_scan_returns_three_pages() {
    let backend = MockScanner::new();
    let options = ScanOptions {
        scanner_id: "mock-feeder-001".to_string(),
        dpi: 72, // Very low DPI for fast test
        color_mode: Default::default(),
        format: Default::default(),
    };

    let progress_pages = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let progress_ref = progress_pages.clone();

    let result = backend.scan(
        &options,
        Box::new(move |page| {
            progress_ref.lock().unwrap().push(page);
        }),
    );

    let pages = result.expect("Scan should succeed");
    assert_eq!(pages.len(), 3);

    let progress = progress_pages.lock().unwrap().clone();
    assert_eq!(progress, vec![1, 2, 3]);

    // Each page should be valid PNG
    for page in &pages {
        assert_eq!(&page.png_data[0..4], &[0x89, 0x50, 0x4E, 0x47]);
    }
}

#[test]
fn scan_dimensions_scale_with_dpi() {
    let backend = MockScanner::new();

    let scan_at = |dpi: u32| -> (u32, u32) {
        let options = ScanOptions {
            scanner_id: "mock-flatbed-001".to_string(),
            dpi,
            color_mode: Default::default(),
            format: Default::default(),
        };
        let pages = backend
            .scan(&options, Box::new(|_| {}))
            .expect("Scan should succeed");
        (pages[0].width, pages[0].height)
    };

    let (w_low, h_low) = scan_at(72);
    let (w_high, h_high) = scan_at(150);

    // Higher DPI should produce larger images
    assert!(w_high > w_low);
    assert!(h_high > h_low);
}
