use crate::helpers::spawn_app;

#[tokio::test]
async fn index_page_returns_200() {
    let app = spawn_app().await;
    let resp = app
        .client
        .get(&app.address)
        .send()
        .await
        .expect("failed to send request");
    assert_eq!(resp.status(), 200);
}

#[tokio::test]
async fn index_page_is_html() {
    let app = spawn_app().await;
    let resp = app
        .client
        .get(&app.address)
        .send()
        .await
        .expect("failed to send request");
    let content_type = resp.headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    assert!(content_type.contains("text/html"), "expected text/html, got: {content_type}");
}

#[tokio::test]
async fn index_page_contains_search_ui() {
    let app = spawn_app().await;
    let body = app
        .client
        .get(&app.address)
        .send()
        .await
        .expect("failed to send request")
        .text()
        .await
        .expect("failed to read body");
    assert!(body.contains("/api/search"), "page should reference the search API");
    assert!(body.contains("/api/ask"), "page should reference the ask API");
    assert!(body.contains("/api/recent"), "page should reference the recent API");
}
