use super::{ScanError, ScanOptions, ScannedPage, ScannerBackend, ScannerInfo, ScannerType};

/// Windows scanner backend using the WIA 2.0 (Windows Image Acquisition) COM API.
///
/// WIA provides access to scanners via:
/// - `IWiaDevMgr2` to enumerate and create scanner devices
/// - `IWiaItem2` to configure and execute scans
///
/// This uses the `windows-rs` crate for COM API bindings.
pub struct WindowsScanner {
    // Will hold IWiaDevMgr2 instance
}

impl WindowsScanner {
    pub fn new() -> Self {
        // TODO: Initialize COM and create IWiaDevMgr2 instance.
        //
        // The flow is:
        // 1. Initialize COM: `CoInitializeEx(None, COINIT_MULTITHREADED)`
        // 2. Create device manager:
        //    `CoCreateInstance(&WiaDevMgr2, None, CLSCTX_LOCAL_SERVER)`
        //
        // Example windows-rs pattern:
        // ```
        // use windows::Win32::Devices::ImageAcquisition::*;
        // use windows::Win32::System::Com::*;
        //
        // unsafe { CoInitializeEx(None, COINIT_MULTITHREADED)? };
        // let mgr: IWiaDevMgr2 = unsafe {
        //     CoCreateInstance(&WiaDevMgr2, None, CLSCTX_LOCAL_SERVER)?
        // };
        // ```
        tracing::info!("Windows scanner backend initialized (stub)");
        WindowsScanner {}
    }
}

impl ScannerBackend for WindowsScanner {
    fn list_scanners(&self) -> Vec<ScannerInfo> {
        // TODO: Enumerate scanner devices via IWiaDevMgr2::EnumDeviceInfo.
        //
        // Implementation steps:
        // 1. Call `EnumDeviceInfo(WIA_DEVINFO_ENUM_LOCAL)` to get IEnumWIA_DEV_INFO
        // 2. Iterate with `Next()`, for each IWiaPropertyStorage:
        //    - Read `WIA_DIP_DEV_ID` -> ScannerInfo.id
        //    - Read `WIA_DIP_DEV_NAME` -> ScannerInfo.name
        //    - Read `WIA_DIP_DEV_TYPE` -> ScannerType
        // 3. Return the list
        tracing::warn!("Windows list_scanners not yet implemented, returning empty list");
        vec![]
    }

    fn scan(
        &self,
        options: &ScanOptions,
        _on_progress: Box<dyn Fn(usize) + Send>,
    ) -> Result<Vec<ScannedPage>, ScanError> {
        // TODO: Perform a scan using WIA.
        //
        // The flow is:
        // 1. Create device: `IWiaDevMgr2::CreateDevice(device_id)`
        // 2. Enumerate child items to find the scanner item (flatbed/feeder)
        // 3. Set scan properties on the item:
        //    - `WIA_IPS_XRES` / `WIA_IPS_YRES` = DPI
        //    - `WIA_IPS_CUR_INTENT` = color mode
        //    - `WIA_IPS_PAGES` = 0 for all pages (ADF) or 1 for single
        //    - `WIA_IPA_FORMAT` = output format GUID
        // 4. Call `IWiaTransfer::Download()` with a callback
        //    - The callback receives image data in `TransferCallback::GetNextStream()`
        //    - For ADF, multiple streams are provided (one per page)
        // 5. Convert received image data to PNG bytes
        let _ = options;
        Err(ScanError::from(
            "Windows scanner backend not yet implemented",
        ))
    }
}
