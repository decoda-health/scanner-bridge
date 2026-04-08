use std::net::SocketAddr;

use futures_util::{SinkExt, StreamExt};
use tokio_tungstenite::{connect_async, tungstenite::Message};

use scanner_bridge::scanner::mock::MockScanner;
use scanner_bridge::server::{build_router, WsEvent};

/// Start the server on a random port and return the address.
async fn start_test_server() -> SocketAddr {
    let app = build_router(Box::new(MockScanner::new()));

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .unwrap();
    let addr = listener.local_addr().unwrap();

    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    addr
}

/// Read the next text message from the WebSocket and parse as WsEvent.
async fn recv_event(
    ws: &mut tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
) -> WsEvent {
    loop {
        let msg = ws.next().await.unwrap().unwrap();
        if let Message::Text(text) = msg {
            return serde_json::from_str(text.as_str()).unwrap();
        }
    }
}

#[tokio::test]
async fn websocket_flatbed_scan_flow() {
    let addr = start_test_server().await;
    let url = format!("ws://{addr}/ws");

    let (mut ws, _) = connect_async(&url).await.expect("Failed to connect");

    // Send scan command
    let cmd = serde_json::json!({
        "action": "scan",
        "scannerId": "mock-flatbed-001",
        "dpi": 72,
        "colorMode": "color",
        "format": "png"
    });
    ws.send(Message::Text(cmd.to_string().into())).await.unwrap();

    // Should receive: scan_started, scan_progress(1), scan_complete
    let event = recv_event(&mut ws).await;
    assert!(matches!(event, WsEvent::ScanStarted));

    let event = recv_event(&mut ws).await;
    assert!(matches!(event, WsEvent::ScanProgress { page: 1 }));

    let event = recv_event(&mut ws).await;
    match event {
        WsEvent::ScanComplete { pages } => {
            assert_eq!(pages.len(), 1);
            assert!(pages[0].data_url.starts_with("data:image/png;base64,"));
            assert!(pages[0].width > 0);
            assert!(pages[0].height > 0);
        }
        other => panic!("Expected ScanComplete, got {other:?}"),
    }

    ws.close(None).await.unwrap();
}

#[tokio::test]
async fn websocket_feeder_scan_returns_pdf() {
    let addr = start_test_server().await;
    let url = format!("ws://{addr}/ws");

    let (mut ws, _) = connect_async(&url).await.expect("Failed to connect");

    // Request PDF format from feeder (3 pages -> combined PDF)
    let cmd = serde_json::json!({
        "action": "scan",
        "scannerId": "mock-feeder-001",
        "dpi": 72,
        "colorMode": "color",
        "format": "pdf"
    });
    ws.send(Message::Text(cmd.to_string().into())).await.unwrap();

    // Collect all events
    let mut events = Vec::new();
    loop {
        let event = recv_event(&mut ws).await;
        let is_terminal = matches!(event, WsEvent::ScanComplete { .. } | WsEvent::ScanError { .. });
        events.push(event);
        if is_terminal {
            break;
        }
    }

    // Should have: scan_started, 3x progress, scan_complete
    assert!(matches!(events[0], WsEvent::ScanStarted));

    let progress_count = events
        .iter()
        .filter(|e| matches!(e, WsEvent::ScanProgress { .. }))
        .count();
    assert_eq!(progress_count, 3);

    match events.last().unwrap() {
        WsEvent::ScanComplete { pages } => {
            // Multi-page feeder with PDF format should combine into 1 PDF
            assert_eq!(pages.len(), 1);
            assert!(pages[0].data_url.starts_with("data:application/pdf;base64,"));
        }
        other => panic!("Expected ScanComplete, got {other:?}"),
    }

    ws.close(None).await.unwrap();
}

#[tokio::test]
async fn websocket_invalid_command_returns_error() {
    let addr = start_test_server().await;
    let url = format!("ws://{addr}/ws");

    let (mut ws, _) = connect_async(&url).await.expect("Failed to connect");

    ws.send(Message::Text("not json".to_string().into()))
        .await
        .unwrap();

    let event = recv_event(&mut ws).await;
    match event {
        WsEvent::Error { message } => {
            assert!(message.contains("Invalid command"));
        }
        other => panic!("Expected Error, got {other:?}"),
    }

    ws.close(None).await.unwrap();
}
