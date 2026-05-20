use crate::helpers::spawn_app;

fn encode(s: &str) -> String {
    percent_encoding::utf8_percent_encode(s, percent_encoding::NON_ALPHANUMERIC).to_string()
}

// --- /api/pages ---

#[tokio::test]
async fn list_pages_returns_all_seeded_entries() {
    let app = spawn_app().await;

    let response = app
        .client
        .get(format!("{}/api/pages", app.address))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 200);
    let items: Vec<serde_json::Value> = response.json().await.unwrap();
    assert_eq!(items.len(), 3);
}

#[tokio::test]
async fn list_pages_filters_by_query() {
    let app = spawn_app().await;

    let response = app
        .client
        .get(format!("{}/api/pages?q=serde", app.address))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 200);
    let items: Vec<serde_json::Value> = response.json().await.unwrap();
    assert!(
        items
            .iter()
            .any(|i| i["url"].as_str().unwrap().contains("serde")),
        "expected serde result"
    );
}

#[tokio::test]
async fn list_pages_respects_limit() {
    let app = spawn_app().await;

    let response = app
        .client
        .get(format!("{}/api/pages?limit=1", app.address))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 200);
    let items: Vec<serde_json::Value> = response.json().await.unwrap();
    assert_eq!(items.len(), 1);
}

// --- /api/starred ---

#[tokio::test]
async fn starred_empty_by_default() {
    let app = spawn_app().await;

    let response = app
        .client
        .get(format!("{}/api/starred", app.address))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 200);
    let items: Vec<serde_json::Value> = response.json().await.unwrap();
    assert!(items.is_empty());
}

// --- /api/star ---

#[tokio::test]
async fn star_and_unstar_page() {
    let app = spawn_app().await;
    let url = "https://rust-lang.org";

    let r = app
        .client
        .post(format!(
            "{}/api/star?url={}&starred=true",
            app.address,
            encode(url)
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 204);

    let items: Vec<serde_json::Value> = app
        .client
        .get(format!("{}/api/starred", app.address))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(
        items.iter().any(|i| i["url"].as_str().unwrap() == url),
        "expected rust-lang.org to be starred"
    );

    let r = app
        .client
        .post(format!(
            "{}/api/star?url={}&starred=false",
            app.address,
            encode(url)
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 204);

    let items: Vec<serde_json::Value> = app
        .client
        .get(format!("{}/api/starred", app.address))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(
        !items.iter().any(|i| i["url"].as_str().unwrap() == url),
        "rust-lang.org should no longer be starred"
    );
}

// --- DELETE /api/page ---

#[tokio::test]
async fn delete_page_removes_entry() {
    let app = spawn_app().await;
    let url = "https://tokio.rs";

    let r = app
        .client
        .delete(format!("{}/api/page?url={}", app.address, encode(url)))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 204);

    let items: Vec<serde_json::Value> = app
        .client
        .get(format!("{}/api/pages", app.address))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(
        !items.iter().any(|i| i["url"].as_str().unwrap() == url),
        "tokio.rs should have been deleted"
    );
}

// --- POST /api/ban ---

#[tokio::test]
async fn ban_host_returns_deleted_count() {
    let app = spawn_app().await;

    let r = app
        .client
        .post(format!("{}/api/ban?host=tokio.rs", app.address))
        .send()
        .await
        .unwrap();

    assert_eq!(r.status(), 200);
    let body: serde_json::Value = r.json().await.unwrap();
    assert!(body["deleted"].is_number());
}

// --- /api/sync/status ---

#[tokio::test]
async fn sync_status_returns_expected_shape() {
    let app = spawn_app().await;

    let r = app
        .client
        .get(format!("{}/api/sync/status", app.address))
        .send()
        .await
        .unwrap();

    assert_eq!(r.status(), 200);
    let body: serde_json::Value = r.json().await.unwrap();
    assert!(body["paused"].is_boolean());
    assert!(body["interval_mins"].is_number());
}

#[tokio::test]
async fn sync_pause_and_unpause() {
    let app = spawn_app().await;

    let r = app
        .client
        .post(format!("{}/api/sync/pause?paused=true", app.address))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 204);

    let body: serde_json::Value = app
        .client
        .get(format!("{}/api/sync/status", app.address))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(body["paused"], true);

    let r = app
        .client
        .post(format!("{}/api/sync/pause?paused=false", app.address))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 204);

    let body: serde_json::Value = app
        .client
        .get(format!("{}/api/sync/status", app.address))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(body["paused"], false);
}

// --- POST /api/reindex ---

#[tokio::test]
async fn reindex_page_returns_202() {
    let app = spawn_app().await;

    let r = app
        .client
        .post(format!(
            "{}/api/reindex?url={}",
            app.address,
            encode("https://rust-lang.org")
        ))
        .send()
        .await
        .unwrap();

    assert_eq!(r.status(), 202);
}

// --- /api/settings ---

#[tokio::test]
async fn get_settings_returns_200_with_object() {
    let app = spawn_app().await;

    let r = app
        .client
        .get(format!("{}/api/settings", app.address))
        .send()
        .await
        .unwrap();

    assert_eq!(r.status(), 200);
    let body: serde_json::Value = r.json().await.unwrap();
    assert!(body.is_object());
}

// --- /api/export/starred ---

#[tokio::test]
async fn export_starred_returns_json_array() {
    let app = spawn_app().await;

    let r = app
        .client
        .get(format!("{}/api/export/starred", app.address))
        .send()
        .await
        .unwrap();

    assert_eq!(r.status(), 200);
    assert!(
        r.headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .contains("application/json")
    );
    let body: Vec<serde_json::Value> = r.json().await.unwrap();
    assert!(body.is_empty());
}

// --- POST /api/import/starred ---

#[tokio::test]
async fn import_starred_roundtrip() {
    let app = spawn_app().await;
    let url = "https://rust-lang.org";

    app.client
        .post(format!(
            "{}/api/star?url={}&starred=true",
            app.address,
            encode(url)
        ))
        .send()
        .await
        .unwrap();

    let export = app
        .client
        .get(format!("{}/api/export/starred", app.address))
        .send()
        .await
        .unwrap()
        .bytes()
        .await
        .unwrap();

    app.client
        .post(format!(
            "{}/api/star?url={}&starred=false",
            app.address,
            encode(url)
        ))
        .send()
        .await
        .unwrap();

    let r = app
        .client
        .post(format!("{}/api/import/starred", app.address))
        .header("content-type", "application/json")
        .body(export)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);

    let items: Vec<serde_json::Value> = app
        .client
        .get(format!("{}/api/starred", app.address))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(
        items.iter().any(|i| i["url"].as_str().unwrap() == url),
        "rust-lang.org should be starred after import"
    );
}
