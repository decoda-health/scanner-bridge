use std::thread;
use std::time::Duration;

use super::{ScanError, ScanOptions, ScannedPage, ScannerBackend, ScannerInfo, ScannerType};

/// A mock scanner backend for development and testing.
/// Returns a generated test image without requiring actual scanner hardware.
pub struct MockScanner;

impl MockScanner {
    pub fn new() -> Self {
        MockScanner
    }

    /// Generate a simple test image with a grid pattern and text label.
    fn generate_test_page(page_number: usize, dpi: u32) -> ScannedPage {
        // A4 at the requested DPI
        let width = (8.5 * dpi as f64) as u32;
        let height = (11.0 * dpi as f64) as u32;

        let mut img = image::RgbImage::new(width, height);

        // White background
        for pixel in img.pixels_mut() {
            *pixel = image::Rgb([255, 255, 255]);
        }

        // Draw a border
        let border = 20u32;
        for x in border..width - border {
            for y in [border, height - border - 1] {
                if let Some(pixel) = img.get_pixel_mut_checked(x, y) {
                    *pixel = image::Rgb([0, 0, 0]);
                }
            }
        }
        for y in border..height - border {
            for x in [border, width - border - 1] {
                if let Some(pixel) = img.get_pixel_mut_checked(x, y) {
                    *pixel = image::Rgb([0, 0, 0]);
                }
            }
        }

        // Draw grid lines every inch
        let grid_spacing = dpi;
        for x in (0..width).step_by(grid_spacing as usize) {
            for y in border..height - border {
                if let Some(pixel) = img.get_pixel_mut_checked(x, y) {
                    *pixel = image::Rgb([200, 200, 200]);
                }
            }
        }
        for y in (0..height).step_by(grid_spacing as usize) {
            for x in border..width - border {
                if let Some(pixel) = img.get_pixel_mut_checked(x, y) {
                    *pixel = image::Rgb([200, 200, 200]);
                }
            }
        }

        // Draw a filled rectangle as a "header" area
        let header_height = dpi / 2;
        for x in border + 1..width - border - 1 {
            for y in border + 1..border + header_height {
                if let Some(pixel) = img.get_pixel_mut_checked(x, y) {
                    *pixel = image::Rgb([41, 98, 255]); // Decoda blue
                }
            }
        }

        // Draw an "X" pattern in the center to make pages visually distinct
        let center_x = width / 2;
        let center_y = height / 2;
        let cross_size = dpi;
        for i in 0..cross_size {
            let thickness = 3u32;
            for t in 0..thickness {
                // Diagonal \
                let x1 = center_x - cross_size / 2 + i;
                let y1 = center_y - cross_size / 2 + i + t;
                if let Some(pixel) = img.get_pixel_mut_checked(x1, y1) {
                    *pixel = image::Rgb([200, 50, 50]);
                }
                // Diagonal /
                let x2 = center_x + cross_size / 2 - i;
                let y2 = center_y - cross_size / 2 + i + t;
                if let Some(pixel) = img.get_pixel_mut_checked(x2, y2) {
                    *pixel = image::Rgb([200, 50, 50]);
                }
            }
        }

        // Encode page number into the pattern by drawing dots
        let dot_y = center_y + cross_size;
        for p in 0..page_number {
            let dot_x = center_x - ((page_number as u32 - 1) * 30) / 2 + (p as u32 * 30);
            for dx in 0..10 {
                for dy in 0..10 {
                    if let Some(pixel) = img.get_pixel_mut_checked(dot_x + dx, dot_y + dy) {
                        *pixel = image::Rgb([41, 98, 255]);
                    }
                }
            }
        }

        // Encode as PNG
        let mut png_data = Vec::new();
        let encoder = image::codecs::png::PngEncoder::new(&mut png_data);
        image::ImageEncoder::write_image(
            encoder,
            img.as_raw(),
            width,
            height,
            image::ExtendedColorType::Rgb8,
        )
        .expect("PNG encoding failed");

        ScannedPage {
            png_data,
            width,
            height,
        }
    }
}

impl ScannerBackend for MockScanner {
    fn list_scanners(&self) -> Vec<ScannerInfo> {
        vec![
            ScannerInfo {
                id: "mock-flatbed-001".to_string(),
                name: "Mock Flatbed Scanner".to_string(),
                scanner_type: ScannerType::Flatbed,
            },
            ScannerInfo {
                id: "mock-feeder-001".to_string(),
                name: "Mock Document Feeder (3 pages)".to_string(),
                scanner_type: ScannerType::Feeder,
            },
        ]
    }

    fn scan(
        &self,
        options: &ScanOptions,
        on_progress: Box<dyn Fn(usize) + Send>,
    ) -> Result<Vec<ScannedPage>, ScanError> {
        let page_count = if options.scanner_id == "mock-feeder-001" {
            3
        } else {
            1
        };

        tracing::info!(
            scanner_id = %options.scanner_id,
            dpi = options.dpi,
            pages = page_count,
            "Mock scan starting"
        );

        let mut pages = Vec::new();

        for i in 0..page_count {
            // Simulate scanning delay (2-4 seconds per page)
            thread::sleep(Duration::from_secs(2));

            on_progress(i + 1);

            let page = Self::generate_test_page(i + 1, options.dpi);
            pages.push(page);

            tracing::info!(page = i + 1, "Mock page scanned");
        }

        Ok(pages)
    }
}
