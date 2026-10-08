use axum::{
    body::Body,
    http::{Request, StatusCode, header},
};
use http_body_util::BodyExt;
use serde_json::json;
use tower::ServiceExt;

#[tokio::test]
async fn info_contract_works_without_web_assets() {
    let directory = tempfile::tempdir().unwrap();
    let response = rig::http::router(directory.path().join("not-built"))
        .oneshot(Request::get("/api/info").body(Body::empty()).unwrap())
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()[header::CONTENT_TYPE], "application/json");
    let body = response.into_body().collect().await.unwrap().to_bytes();
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&body).unwrap(),
        json!({ "name": "Rig", "version": env!("CARGO_PKG_VERSION") })
    );
}

#[tokio::test]
async fn serves_the_index_and_nested_assets_from_the_selected_directory() {
    let directory = tempfile::tempdir().unwrap();
    std::fs::write(directory.path().join("index.html"), "<h1>Rig</h1>").unwrap();
    std::fs::create_dir(directory.path().join("assets")).unwrap();
    std::fs::write(
        directory.path().join("assets/app.js"),
        "console.log('Rig');",
    )
    .unwrap();
    let router = rig::http::router(directory.path());

    for (path, content_type, expected_body) in [
        ("/", "text/html", "<h1>Rig</h1>"),
        ("/assets/app.js", "text/javascript", "console.log('Rig');"),
    ] {
        let response = router
            .clone()
            .oneshot(Request::get(path).body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers()[header::CONTENT_TYPE], content_type);
        let body = response.into_body().collect().await.unwrap().to_bytes();
        assert_eq!(&body[..], expected_body.as_bytes());
    }
}

#[tokio::test]
async fn unknown_paths_are_not_replaced_with_the_index() {
    let directory = tempfile::tempdir().unwrap();
    std::fs::write(directory.path().join("index.html"), "<h1>Rig</h1>").unwrap();
    let response = rig::http::router(directory.path())
        .oneshot(Request::get("/unknown").body(Body::empty()).unwrap())
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn info_rejects_write_requests() {
    let directory = tempfile::tempdir().unwrap();
    let response = rig::http::router(directory.path())
        .oneshot(Request::post("/api/info").body(Body::empty()).unwrap())
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::METHOD_NOT_ALLOWED);
}
