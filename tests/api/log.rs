use crate::helpers::spawn_app;

#[tokio::test]
async fn log_page_returns_200() {
    let app = spawn_app().await;

    let response = app
        .client
        .get(format!("{}/log", app.address))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 200);
}

#[tokio::test]
async fn log_api_returns_200_and_array() {
    let app = spawn_app().await;

    let response = app
        .client
        .get(format!("{}/api/log", app.address))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 200);
    let entries: Vec<serde_json::Value> = response.json().await.unwrap();
    // Fresh app has no log entries yet.
    assert!(entries.is_empty());
}

#[tokio::test]
async fn log_api_entries_appear_after_search() {
    let app = spawn_app().await;

    app.client
        .get(format!("{}/api/search?q=rust", app.address))
        .send()
        .await
        .unwrap();

    let response = app
        .client
        .get(format!("{}/api/log", app.address))
        .send()
        .await
        .unwrap();

    let entries: Vec<serde_json::Value> = response.json().await.unwrap();
    assert!(!entries.is_empty(), "expected a log entry after search");

    let entry = &entries[0];
    assert_eq!(entry["kind"].as_str().unwrap(), "search");
    assert!(entry["message"].as_str().unwrap().contains("rust"));
    assert!(entry["ts"].is_string());
}

#[tokio::test]
async fn log_api_kind_filter_returns_only_matching_entries() {
    let app = spawn_app().await;

    app.client
        .get(format!("{}/api/search?q=rust", app.address))
        .send()
        .await
        .unwrap();

    let response = app
        .client
        .get(format!("{}/api/log?kind=search", app.address))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 200);
    let entries: Vec<serde_json::Value> = response.json().await.unwrap();
    assert!(!entries.is_empty());
    for e in &entries {
        assert_eq!(e["kind"].as_str().unwrap(), "search");
    }
}

#[tokio::test]
async fn log_api_unknown_kind_returns_empty_array() {
    let app = spawn_app().await;

    let response = app
        .client
        .get(format!("{}/api/log?kind=bogus", app.address))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 200);
    let entries: Vec<serde_json::Value> = response.json().await.unwrap();
    assert!(entries.is_empty());
}
