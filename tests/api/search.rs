use crate::helpers::spawn_app;

#[tokio::test]
async fn search_returns_200_for_valid_query() {
    let app = spawn_app().await;

    let response = app
        .client
        .get(format!("{}/api/search?q=rust", app.address))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 200);
}

#[tokio::test]
async fn search_returns_matching_results() {
    let app = spawn_app().await;

    let response = app
        .client
        .get(format!("{}/api/search?q=rust", app.address))
        .send()
        .await
        .unwrap();

    let items: Vec<serde_json::Value> = response.json().await.unwrap();
    assert!(!items.is_empty(), "expected results for 'rust'");
    assert!(items[0]["url"].as_str().unwrap().contains("rust-lang.org"));
}

#[tokio::test]
async fn search_returns_empty_for_no_match() {
    let app = spawn_app().await;

    let response = app
        .client
        .get(format!("{}/api/search?q=zzznomatch99999", app.address))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 200);
    let items: Vec<serde_json::Value> = response.json().await.unwrap();
    assert!(items.is_empty());
}

#[tokio::test]
async fn search_requires_q_param() {
    let app = spawn_app().await;

    let response = app
        .client
        .get(format!("{}/api/search", app.address))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 400);
}

#[tokio::test]
async fn search_results_have_expected_fields() {
    let app = spawn_app().await;

    let response = app
        .client
        .get(format!("{}/api/search?q=rust", app.address))
        .send()
        .await
        .unwrap();

    let items: Vec<serde_json::Value> = response.json().await.unwrap();
    assert!(!items.is_empty());

    let first = &items[0];
    assert!(first["url"].is_string());
    assert!(first["title"].is_string());
    assert!(first["snippet"].is_string());
    assert!(first["rank"].is_number());
}

#[tokio::test]
async fn search_respects_limit_param() {
    let app = spawn_app().await;

    let response = app
        .client
        .get(format!("{}/api/search?q=rust&limit=1", app.address))
        .send()
        .await
        .unwrap();

    let items: Vec<serde_json::Value> = response.json().await.unwrap();
    assert!(items.len() <= 1);
}
