use std::sync::Arc;

use axum::{
    extract::{
        ws::{Message, WebSocket},
        State, WebSocketUpgrade,
    },
    http::Method,
    response::IntoResponse,
    routing::get,
    Json, Router,
};
use serde::{Deserialize, Serialize};
use tower_http::cors::{AllowOrigin, CorsLayer};

use crate::scanner::{OutputFormat, ScanOptions, ScannerBackend, ScannerInfo};

/// Shared application state passed to all handlers.
pub struct AppState {
    pub backend: Box<dyn ScannerBackend>,
}

/// Start the HTTP + WebSocket server on localhost.
pub async fn start_server(backend: Box<dyn ScannerBackend>, port: u16) -> Result<(), String> {
    let state = Arc::new(AppState { backend });

    let cors = CorsLayer::new()
        .allow_origin(AllowOrigin::list([
            "http://localhost:3000".parse().unwrap(),
            "http://localhost:3001".parse().unwrap(),
            "https://app.decoda.com".parse().unwrap(),
        ]))
        .allow_methods([Method::GET, Method::POST, Method::OPTIONS])
        .allow_headers(tower_http::cors::Any);

    let app = Router::new()
        .route("/health", get(health))
        .route("/scanners", get(list_scanners))
        .route("/ws", get(ws_upgrade))
        .layer(cors)
        .with_state(state);

    let addr = format!("127.0.0.1:{port}");
    let listener = tokio::net::TcpListener::bind(&addr)
        .await
        .map_err(|e| format!("Failed to bind to {addr}: {e}"))?;

    tracing::info!("Scanner bridge server listening on {addr}");

    axum::serve(listener, app)
        .await
        .map_err(|e| format!("Server error: {e}"))
}

// ---------------------------------------------------------------------------
// HTTP Handlers
// ---------------------------------------------------------------------------

#[derive(Serialize)]
struct HealthResponse {
    status: &'static str,
    version: &'static str,
}

async fn health() -> Json<HealthResponse> {
    Json(HealthResponse {
        status: "ok",
        version: env!("CARGO_PKG_VERSION"),
    })
}

async fn list_scanners(State(state): State<Arc<AppState>>) -> Json<Vec<ScannerInfo>> {
    let scanners = state.backend.list_scanners();
    Json(scanners)
}

// ---------------------------------------------------------------------------
// WebSocket Handler
// ---------------------------------------------------------------------------

async fn ws_upgrade(
    State(state): State<Arc<AppState>>,
    ws: WebSocketUpgrade,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_ws(socket, state))
}

/// Messages the client can send over the WebSocket.
#[derive(Debug, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
enum WsCommand {
    Scan(ScanOptions),
}

/// Messages the server sends back over the WebSocket.
#[derive(Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum WsEvent {
    ScanStarted,
    ScanProgress { page: usize },
    ScanComplete { pages: Vec<PageData> },
    ScanError { message: String },
    Error { message: String },
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct PageData {
    /// Base64-encoded image data as a data URL
    data_url: String,
    width: u32,
    height: u32,
}

async fn handle_ws(mut socket: WebSocket, state: Arc<AppState>) {
    tracing::info!("WebSocket client connected");

    while let Some(Ok(msg)) = socket.recv().await {
        match msg {
            Message::Text(text) => {
                let command: WsCommand = match serde_json::from_str(&text) {
                    Ok(cmd) => cmd,
                    Err(e) => {
                        let _ = send_event(
                            &mut socket,
                            WsEvent::Error {
                                message: format!("Invalid command: {e}"),
                            },
                        )
                        .await;
                        continue;
                    }
                };

                match command {
                    WsCommand::Scan(options) => {
                        handle_scan(&mut socket, &state, options).await;
                    }
                }
            }
            Message::Close(_) => {
                tracing::info!("WebSocket client disconnected");
                break;
            }
            _ => {}
        }
    }
}

async fn handle_scan(socket: &mut WebSocket, state: &Arc<AppState>, options: ScanOptions) {
    // Notify client that scanning has started
    if send_event(socket, WsEvent::ScanStarted).await.is_err() {
        return;
    }

    // Create a channel for progress updates from the blocking scan thread
    let (progress_tx, mut progress_rx) = tokio::sync::mpsc::unbounded_channel::<usize>();

    // Run the blocking scan in a separate thread
    let scan_options = options.clone();
    let state = Arc::clone(state);
    let mut scan_handle = tokio::task::spawn_blocking(move || {
        let on_progress = Box::new(move |page: usize| {
            let _ = progress_tx.send(page);
        });
        state.backend.scan(&scan_options, on_progress)
    });

    // Forward progress events while waiting for the scan to complete
    loop {
        tokio::select! {
            biased;

            Some(page) = progress_rx.recv() => {
                if send_event(socket, WsEvent::ScanProgress { page }).await.is_err() {
                    return;
                }
            }
            result = &mut scan_handle => {
                // Drain remaining progress events
                while let Ok(page) = progress_rx.try_recv() {
                    if send_event(socket, WsEvent::ScanProgress { page }).await.is_err() {
                        return;
                    }
                }

                match result {
                    Ok(Ok(scanned_pages)) => {
                        // If PDF format requested and multiple pages, combine into PDF
                        if matches!(options.format, OutputFormat::Pdf) && scanned_pages.len() > 1 {
                            match crate::pdf::pages_to_pdf(&scanned_pages, options.dpi) {
                                Ok(pdf_bytes) => {
                                    let b64 = base64::Engine::encode(
                                        &base64::engine::general_purpose::STANDARD,
                                        &pdf_bytes,
                                    );
                                    let pages = vec![PageData {
                                        data_url: format!("data:application/pdf;base64,{b64}"),
                                        width: scanned_pages[0].width,
                                        height: scanned_pages[0].height,
                                    }];
                                    let _ = send_event(socket, WsEvent::ScanComplete { pages }).await;
                                }
                                Err(e) => {
                                    let _ = send_event(socket, WsEvent::ScanError {
                                        message: format!("PDF assembly failed: {e}"),
                                    }).await;
                                }
                            }
                        } else {
                            let pages: Vec<PageData> = scanned_pages
                                .into_iter()
                                .map(|page| {
                                    let b64 = base64::Engine::encode(
                                        &base64::engine::general_purpose::STANDARD,
                                        &page.png_data,
                                    );
                                    PageData {
                                        data_url: format!("data:image/png;base64,{b64}"),
                                        width: page.width,
                                        height: page.height,
                                    }
                                })
                                .collect();
                            let _ = send_event(socket, WsEvent::ScanComplete { pages }).await;
                        }
                    }
                    Ok(Err(scan_err)) => {
                        let _ = send_event(socket, WsEvent::ScanError {
                            message: scan_err.message,
                        }).await;
                    }
                    Err(join_err) => {
                        let _ = send_event(socket, WsEvent::ScanError {
                            message: format!("Scan thread panicked: {join_err}"),
                        }).await;
                    }
                }
                return;
            }
        }
    }
}

async fn send_event(socket: &mut WebSocket, event: WsEvent) -> Result<(), ()> {
    let json = serde_json::to_string(&event).map_err(|_| ())?;
    socket.send(Message::Text(json.into())).await.map_err(|e| {
        tracing::warn!("Failed to send WebSocket message: {e}");
    })
}
