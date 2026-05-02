use crate::helpers::spawn_app;

#[tokio::test]
async fn ask_missing_q_returns_400() {
    let app = spawn_app().await;
    let resp = app
        .client
        .get(format!("{}/api/ask", app.address))
        .send()
        .await
        .expect("failed to send request");
    assert_eq!(resp.status(), 400);
}

#[tokio::test]
async fn ask_matching_query_returns_200_with_answer_and_sources() {
    let app = spawn_app().await;
    let resp = app
        .client
        .get(format!("{}/api/ask", app.address))
        .query(&[("q", "rust programming")])
        .send()
        .await
        .expect("failed to send request");
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.expect("failed to parse JSON");
    assert_eq!(
        body["answer"].as_str().unwrap(),
        "Mock answer from LLM.",
        "answer should be the mock LLM response"
    );
    let sources = body["sources"].as_array().unwrap();
    assert!(!sources.is_empty(), "should return at least one source URL");
    let source_urls: Vec<&str> = sources.iter().map(|s| s.as_str().unwrap()).collect();
    assert!(
        source_urls.contains(&"https://rust-lang.org"),
        "rust-lang.org should be a source"
    );
}

#[tokio::test]
async fn ask_no_matching_pages_returns_no_relevant_message() {
    let app = spawn_app().await;
    let resp = app
        .client
        .get(format!("{}/api/ask", app.address))
        .query(&[("q", "zzz_nonexistent_topic_xyz")])
        .send()
        .await
        .expect("failed to send request");
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.expect("failed to parse JSON");
    assert!(
        body["answer"].as_str().unwrap().contains("No relevant pages found"),
        "answer should indicate no pages found"
    );
    assert!(
        body["sources"].as_array().unwrap().is_empty(),
        "sources should be empty"
    );
}

#[tokio::test]
async fn ask_response_has_expected_fields() {
    let app = spawn_app().await;
    let resp = app
        .client
        .get(format!("{}/api/ask", app.address))
        .query(&[("q", "tokio async")])
        .send()
        .await
        .expect("failed to send request");
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.expect("failed to parse JSON");
    assert!(body["answer"].is_string(), "response must have an 'answer' string field");
    assert!(body["sources"].is_array(), "response must have a 'sources' array field");
}
