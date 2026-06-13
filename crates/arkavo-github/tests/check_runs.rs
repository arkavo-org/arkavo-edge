use arkavo_github::{CheckRunDetails, GitHubOperations};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test]
async fn create_check_run_returns_id() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/repos/o/r/check-runs"))
        .respond_with(ResponseTemplate::new(201).set_body_json(serde_json::json!({ "id": 999 })))
        .mount(&server)
        .await;
    let ops = GitHubOperations::with_base_url("t", &server.uri()).unwrap();
    let id = ops
        .create_check_run(
            "o",
            "r",
            "arkavo-eval/gemma-4-12b",
            "deadbeef",
            "completed",
            CheckRunDetails {
                conclusion: Some("success"),
                output_title: Some("Eval passed"),
                output_summary: Some("ok"),
            },
        )
        .await
        .unwrap();
    assert_eq!(id, 999);
}

#[tokio::test]
async fn update_check_run_ok() {
    let server = MockServer::start().await;
    Mock::given(method("PATCH"))
        .and(path("/repos/o/r/check-runs/999"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({ "id": 999 })))
        .mount(&server)
        .await;
    let ops = GitHubOperations::with_base_url("t", &server.uri()).unwrap();
    ops.update_check_run(
        "o",
        "r",
        999,
        "completed",
        CheckRunDetails {
            conclusion: Some("failure"),
            output_title: Some("Regression"),
            output_summary: Some("similarity 0.5 < 0.87"),
        },
    )
    .await
    .unwrap();
}
