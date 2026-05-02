use crate::helpers::spawn_app;

#[tokio::test]
async fn health_check_returns_200() {
    let app = spawn_app().await;

    let response = app
        .client
        .get(format!("{}/health", app.address))
        .send()
        .await
        .expect("failed to execute request");

    assert!(response.status().is_success());
}

#[tokio::test]
async fn health_check_has_no_body() {
    let app = spawn_app().await;

    let response = app
        .client
        .get(format!("{}/health", app.address))
        .send()
        .await
        .unwrap();

    let body = response.bytes().await.unwrap();
    assert!(body.is_empty());
}
