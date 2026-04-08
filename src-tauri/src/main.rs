// Prevents an additional console window on Windows in release mode.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod pdf;
mod scanner;
mod server;

use tauri::{
    menu::{Menu, MenuItem},
    tray::TrayIconBuilder,
    Manager,
};

const DEFAULT_PORT: u16 = 11235;

fn main() {
    tracing_subscriber::fmt::init();

    // Check CLI args for --mock flag
    let use_mock = std::env::args().any(|arg| arg == "--mock");
    let port = std::env::args()
        .position(|arg| arg == "--port")
        .and_then(|i| std::env::args().nth(i + 1))
        .and_then(|p| p.parse().ok())
        .unwrap_or(DEFAULT_PORT);

    // Create the scanner backend
    let backend = scanner::create_backend(use_mock);

    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .setup(move |app| {
            // Build the system tray menu
            let show = MenuItem::with_id(app, "show", "Show", true, None::<&str>)?;
            let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&show, &quit])?;

            TrayIconBuilder::new()
                .menu(&menu)
                .tooltip("Scanner Bridge")
                .on_menu_event(|app, event| match event.id.as_ref() {
                    "show" => {
                        if let Some(window) = app.get_webview_window("main") {
                            let _ = window.show();
                            let _ = window.set_focus();
                        }
                    }
                    "quit" => {
                        app.exit(0);
                    }
                    _ => {}
                })
                .build(app)?;

            // Start the HTTP/WebSocket server in the background
            let rt = tokio::runtime::Runtime::new().expect("Failed to create tokio runtime");
            std::thread::spawn(move || {
                rt.block_on(async {
                    if let Err(e) = server::start_server(backend, port).await {
                        tracing::error!("Server failed: {e}");
                    }
                });
            });

            tracing::info!("Scanner Bridge started on port {port}");

            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
