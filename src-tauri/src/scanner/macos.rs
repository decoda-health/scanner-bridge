use super::{ScanError, ScanOptions, ScannedPage, ScannerBackend, ScannerInfo};

/// macOS scanner backend using the ImageCaptureCore framework.
///
/// ImageCaptureCore provides access to scanners via:
/// - `ICDeviceBrowser` to discover scanner devices
/// - `ICScannerDevice` to control scanning
///
/// This uses the `objc2` crate for Objective-C FFI bindings.
pub struct MacOsScanner {
    // Will hold ICDeviceBrowser and discovered devices
}

impl MacOsScanner {
    pub fn new() -> Self {
        // TODO: Initialize ICDeviceBrowser and start browsing for devices.
        //
        // The flow is:
        // 1. Create an ICDeviceBrowser instance
        // 2. Set a delegate that receives `didAddDevice` / `didRemoveDevice` callbacks
        // 3. Call `start()` to begin discovery
        // 4. Maintain a list of discovered ICScannerDevice instances
        //
        // Key challenge: ICDeviceBrowser uses delegate callbacks on the main thread,
        // but our server runs on a tokio runtime. We'll need to bridge between the
        // two using channels or a shared mutex.
        //
        // Example objc2 pattern:
        // ```
        // use objc2::rc::Retained;
        // use objc2_foundation::NSObject;
        //
        // // Load the ImageCaptureCore framework
        // let browser: Retained<NSObject> = unsafe {
        //     let cls = objc2::class!(ICDeviceBrowser);
        //     msg_send![cls, new]
        // };
        // ```
        tracing::info!("macOS scanner backend initialized (stub)");
        MacOsScanner {}
    }
}

impl ScannerBackend for MacOsScanner {
    fn list_scanners(&self) -> Vec<ScannerInfo> {
        // TODO: Return the list of scanners discovered by ICDeviceBrowser.
        //
        // For each ICScannerDevice, extract:
        // - `name` property -> ScannerInfo.name
        // - `UUIDString` property -> ScannerInfo.id
        // - `documentType` property -> ScannerType (flatbed vs feeder)
        //
        // Implementation steps:
        // 1. Lock the shared device list
        // 2. Map each ICScannerDevice to a ScannerInfo
        // 3. Return the list
        tracing::warn!("macOS list_scanners not yet implemented, returning empty list");
        vec![]
    }

    fn scan(
        &self,
        options: &ScanOptions,
        _on_progress: Box<dyn Fn(usize) + Send>,
    ) -> Result<Vec<ScannedPage>, ScanError> {
        // TODO: Perform a scan using ICScannerDevice.
        //
        // The flow is:
        // 1. Find the device matching options.scanner_id
        // 2. Open a session: `requestOpenSession`
        // 3. Select the functional unit (flatbed or document feeder)
        // 4. Configure scan settings:
        //    - Resolution (DPI) via `physicalSize` and `preferredResolutions`
        //    - Color mode via `pixelDataType` (RGB, Gray, BW)
        //    - Scan area via `scanArea` (default: full bed)
        // 5. Call `requestScan` and wait for delegate callbacks:
        //    - `didScanToURL:` receives the scanned image file
        //    - `didCompleteWithError:` signals completion
        // 6. Read the image files, convert to PNG bytes
        // 7. Close session: `requestCloseSession`
        //
        // For ADF (document feeder):
        // - Set the document feeder as the selected functional unit
        // - The scanner will automatically feed pages until the feeder is empty
        // - Each page triggers a separate `didScanToURL:` callback
        let _ = options;
        Err(ScanError::from(
            "macOS scanner backend not yet implemented",
        ))
    }
}
