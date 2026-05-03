use crate::helpers::spawn_app;

#[tokio::test]
async fn recent_returns_200_with_json_array() {
    let app = spawn_app().await;

    let response = app
        .client
        .get(format!("{}/api/recent", app.address))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 200);
    let items: Vec<serde_json::Value> = response.json().await.unwrap();
    assert!(!items.is_empty());
}

#[tokio::test]
async fn recent_items_have_expected_fields() {
    let app = spawn_app().await;

    let response = app
        .client
        .get(format!("{}/api/recent?limit=1", app.address))
        .send()
        .await
        .unwrap();

    let items: Vec<serde_json::Value> = response.json().await.unwrap();
    let item = &items[0];

    assert!(item["id"].is_number());
    assert!(item["url"].is_string());
    assert!(item["host"].is_string());
    assert!(item["last_visit_time"].is_string());
    assert!(item["visit_count"].is_number());
}

#[tokio::test]
async fn recent_is_ordered_by_last_visit_desc() {
    let app = spawn_app().await;

    let response = app
        .client
        .get(format!("{}/api/recent?limit=5", app.address))
        .send()
        .await
        .unwrap();

    let items: Vec<serde_json::Value> = response.json().await.unwrap();
    let times: Vec<&str> = items
        .iter()
        .map(|i| i["last_visit_time"].as_str().unwrap())
        .collect();

    let mut sorted = times.clone();
    sorted.sort_by(|a, b| b.cmp(a));
    assert_eq!(times, sorted, "items must be newest-first");
}

#[tokio::test]
async fn recent_most_recent_item_is_correct() {
    let app = spawn_app().await;

    let response = app
        .client
        .get(format!("{}/api/recent?limit=1", app.address))
        .send()
        .await
        .unwrap();

    let items: Vec<serde_json::Value> = response.json().await.unwrap();
    assert_eq!(
        items[0]["url"].as_str().unwrap(),
        "https://example.com/page1"
    );
}

#[tokio::test]
async fn recent_respects_limit_param() {
    let app = spawn_app().await;

    let response = app
        .client
        .get(format!("{}/api/recent?limit=3", app.address))
        .send()
        .await
        .unwrap();

    let items: Vec<serde_json::Value> = response.json().await.unwrap();
    assert_eq!(items.len(), 3);
}

#[tokio::test]
async fn recent_returns_all_when_limit_exceeds_total() {
    let app = spawn_app().await;

    // test DB has 5 items; default limit is 20
    let response = app
        .client
        .get(format!("{}/api/recent", app.address))
        .send()
        .await
        .unwrap();

    let items: Vec<serde_json::Value> = response.json().await.unwrap();
    assert_eq!(items.len(), 5);
}
