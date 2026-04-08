use printpdf::{
    Mm, Op, PdfDocument, PdfPage, PdfSaveOptions, RawImage, RawImageData, RawImageFormat,
    XObjectTransform,
};

use crate::scanner::ScannedPage;

/// Combine multiple scanned pages into a single PDF document.
/// Returns the PDF as bytes.
pub fn pages_to_pdf(pages: &[ScannedPage], dpi: u32) -> Result<Vec<u8>, String> {
    if pages.is_empty() {
        return Err("No pages to convert".to_string());
    }

    let mut doc = PdfDocument::new("Scanned Document");

    let pdf_pages: Vec<PdfPage> = pages
        .iter()
        .map(|page| {
            // Convert pixels to mm using the scan DPI
            let width_mm = Mm(page.width as f32 * 25.4 / dpi as f32);
            let height_mm = Mm(page.height as f32 * 25.4 / dpi as f32);

            // Decode PNG to raw RGB pixels
            let img = ::image::load_from_memory(&page.png_data)
                .map_err(|e| format!("Failed to decode PNG: {e}"))?;
            let rgb = img.to_rgb8();

            let raw_image = RawImage {
                pixels: RawImageData::U8(rgb.into_raw()),
                width: page.width as usize,
                height: page.height as usize,
                data_format: RawImageFormat::RGB8,
                tag: Vec::new(),
            };

            let image_id = doc.add_image(&raw_image);

            let ops = vec![Op::UseXobject {
                id: image_id,
                transform: XObjectTransform {
                    dpi: Some(dpi as f32),
                    ..Default::default()
                },
            }];

            Ok(PdfPage::new(width_mm, height_mm, ops))
        })
        .collect::<Result<Vec<_>, String>>()?;

    doc.with_pages(pdf_pages);

    let mut warnings = Vec::new();
    let bytes = doc.save(&PdfSaveOptions::default(), &mut warnings);

    Ok(bytes)
}
