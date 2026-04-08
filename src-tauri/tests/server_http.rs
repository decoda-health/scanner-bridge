use axum::body::Body;
use http_body_util::BodyExt;
use hyper::Request;
use tower::ServiceExt;

use scanner_bridge::scanner::mock::MockScanner;
use scanner_bridge::server::build_router;

fn mock_app() -> axum::Router {
    build_router(Box::new(MockScanner::new()))
}

#[tokio::test]
async fn health_endpoint_returns_ok() {
    let app = mock_app();

    let response = app
        .oneshot(
            Request::builder()
                .uri("/health")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), 200);

    let body = response.into_body().collect().await.unwrap().to_bytes();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();

    assert_eq!(json["status"], "ok");
    assert_eq!(json["version"], env!("CARGO_PKG_VERSION"));
}

#[tokio::test]
async fn scanners_endpoint_returns_mock_devices() {
    let app = mock_app();

    let response = app
        .oneshot(
            Request::builder()
                .uri("/scanners")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), 200);

    let body = response.into_body().collect().await.unwrap().to_bytes();
    let scanners: Vec<serde_json::Value> = serde_json::from_slice(&body).unwrap();

    assert_eq!(scanners.len(), 2);
    assert_eq!(scanners[0]["id"], "mock-flatbed-001");
    assert_eq!(scanners[0]["name"], "Mock Flatbed Scanner");
    assert_eq!(scanners[0]["scanner_type"], "flatbed");
    assert_eq!(scanners[1]["id"], "mock-feeder-001");
    assert_eq!(scanners[1]["scanner_type"], "feeder");
}

#[tokio::test]
async fn unknown_route_returns_404() {
    let app = mock_app();

    let response = app
        .oneshot(
            Request::builder()
                .uri("/nonexistent")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), 404);
}
