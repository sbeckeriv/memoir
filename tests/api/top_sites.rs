use crate::helpers::spawn_app;

#[tokio::test]
async fn top_sites_returns_200_with_json_array() {
    let app = spawn_app().await;

    let response = app
        .client
        .get(format!("{}/api/top-sites", app.address))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 200);
    let items: Vec<serde_json::Value> = response.json().await.unwrap();
    assert!(!items.is_empty());
}

#[tokio::test]
async fn top_sites_is_ordered_by_visit_count_desc() {
    let app = spawn_app().await;

    let response = app
        .client
        .get(format!("{}/api/top-sites?limit=5", app.address))
        .send()
        .await
        .unwrap();

    let items: Vec<serde_json::Value> = response.json().await.unwrap();
    let counts: Vec<i64> = items
        .iter()
        .map(|i| i["visit_count"].as_i64().unwrap())
        .collect();

    let mut sorted = counts.clone();
    sorted.sort_by(|a, b| b.cmp(a));
    assert_eq!(counts, sorted, "items must be highest-visit-count first");
}

#[tokio::test]
async fn top_sites_first_is_most_visited() {
    let app = spawn_app().await;

    let response = app
        .client
        .get(format!("{}/api/top-sites?limit=1", app.address))
        .send()
        .await
        .unwrap();

    let items: Vec<serde_json::Value> = response.json().await.unwrap();
    // github.com has 50 visits in the test fixture
    assert_eq!(items[0]["host"].as_str().unwrap(), "github.com");
    assert_eq!(items[0]["visit_count"].as_i64().unwrap(), 50);
}

#[tokio::test]
async fn top_sites_respects_limit_param() {
    let app = spawn_app().await;

    let response = app
        .client
        .get(format!("{}/api/top-sites?limit=2", app.address))
        .send()
        .await
        .unwrap();

    let items: Vec<serde_json::Value> = response.json().await.unwrap();
    assert_eq!(items.len(), 2);
}
