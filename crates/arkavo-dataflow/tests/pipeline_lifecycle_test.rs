#![allow(clippy::disallowed_methods)]

use arkavo_dataflow::dsl::{Blueprint, Condition, ConditionOperator, Link, Node, Rule};
use arkavo_dataflow::{DataflowConfig, DataflowEngine};
use arkavo_test_macros::spec;

fn echo_blueprint(name: &str) -> Blueprint {
    let mut blueprint = Blueprint::new(name);
    blueprint.add_node(Node::source("input"));
    blueprint.add_node(Node::sink("output"));
    blueprint.add_link(Link::new("input", "output"));
    blueprint
}

fn filtered_blueprint(name: &str) -> Blueprint {
    let mut blueprint = echo_blueprint(name);
    blueprint.links[0] = Link::new("input", "output").with_rule(Rule::filter(vec![Condition {
        field: "message".to_string(),
        operator: ConditionOperator::Contains,
        value: serde_json::json!("error"),
    }]));
    blueprint
}

#[spec("DATA-001")]
#[tokio::test]
async fn create_pipeline_from_blueprint_returns_unique_ids() {
    let engine = DataflowEngine::new(DataflowConfig::default());

    let first = engine
        .create_pipeline_from_blueprint(echo_blueprint("first"))
        .unwrap();
    let second = engine
        .create_pipeline_from_blueprint(echo_blueprint("second"))
        .unwrap();

    assert_ne!(first, second);
    assert_eq!(first.get_version_num(), 4);
    assert_eq!(second.get_version_num(), 4);
}

#[spec("DATA-001")]
#[tokio::test]
async fn create_pipeline_from_blueprint_stores_pipeline_in_engine() {
    let engine = DataflowEngine::new(DataflowConfig::default());

    let id = engine
        .create_pipeline_from_blueprint(echo_blueprint("stored"))
        .unwrap();

    let pipeline = engine.get_pipeline(id).expect("pipeline must be stored");
    assert_eq!(pipeline.id(), id);
    assert_eq!(pipeline.name(), "stored");

    let listed = engine.list_pipelines();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0], (id, "stored".to_string()));
}

#[spec("DATA-002")]
#[tokio::test]
async fn create_pipeline_from_nl_connects_source_to_sink() {
    let engine = DataflowEngine::new(DataflowConfig::default());

    let id = engine
        .create_pipeline_from_nl("Connect input to output")
        .unwrap();

    let pipeline = engine.get_pipeline(id).expect("NL pipeline must be stored");
    let blueprint = pipeline.to_blueprint();
    assert_eq!(blueprint.nodes.len(), 2);
    assert_eq!(blueprint.links.len(), 1);
    assert_eq!(blueprint.links[0].from, "input");
    assert_eq!(blueprint.links[0].to, "output");
}

#[spec("DATA-002")]
#[tokio::test]
async fn create_pipeline_from_nl_applies_parsed_filter_condition() {
    let engine = DataflowEngine::new(DataflowConfig::default());

    let id = engine
        .create_pipeline_from_nl("Connect input to output when message contains 'error'")
        .unwrap();

    let pipeline = engine.get_pipeline(id).expect("NL pipeline must be stored");
    let blueprint = pipeline.to_blueprint();
    let rule = blueprint.links[0]
        .rule
        .as_ref()
        .expect("filter rule must be generated from the NL condition");
    let conditions = rule.conditions.as_ref().expect("filter conditions");
    assert_eq!(conditions.len(), 1);
    assert_eq!(conditions[0].field, "message");
    assert_eq!(conditions[0].operator, ConditionOperator::Contains);
}

#[spec("DATA-003")]
#[tokio::test]
async fn start_pipeline_transitions_to_running() {
    let engine = DataflowEngine::new(DataflowConfig::default());
    let id = engine
        .create_pipeline_from_blueprint(echo_blueprint("startable"))
        .unwrap();

    engine.start_pipeline(id).await.unwrap();

    let pipeline = engine.get_pipeline(id).unwrap();
    assert_eq!(
        pipeline.status().await,
        arkavo_dataflow::engine::PipelineStatus::Running
    );

    engine.stop_pipeline(id).await.unwrap();
}

#[spec("DATA-003")]
#[tokio::test]
async fn start_pipeline_errors_when_pipeline_not_found() {
    let engine = DataflowEngine::new(DataflowConfig::default());

    let err = engine
        .start_pipeline(uuid::Uuid::new_v4())
        .await
        .unwrap_err();

    assert_eq!(err.to_string(), "Pipeline not found");
}

#[spec("DATA-004")]
#[tokio::test]
async fn stop_pipeline_transitions_running_pipeline_to_stopped() {
    let engine = DataflowEngine::new(DataflowConfig::default());
    let id = engine
        .create_pipeline_from_blueprint(echo_blueprint("stoppable"))
        .unwrap();
    engine.start_pipeline(id).await.unwrap();

    engine.stop_pipeline(id).await.unwrap();

    let pipeline = engine.get_pipeline(id).unwrap();
    assert_eq!(
        pipeline.status().await,
        arkavo_dataflow::engine::PipelineStatus::Stopped
    );
}

#[spec("DATA-004")]
#[tokio::test]
async fn stop_pipeline_rejects_non_running_and_unknown_pipelines() {
    let engine = DataflowEngine::new(DataflowConfig::default());
    let id = engine
        .create_pipeline_from_blueprint(echo_blueprint("never-started"))
        .unwrap();

    let err = engine.stop_pipeline(id).await.unwrap_err();
    assert_eq!(err.to_string(), "Pipeline is not running");

    let err = engine
        .stop_pipeline(uuid::Uuid::new_v4())
        .await
        .unwrap_err();
    assert_eq!(err.to_string(), "Pipeline not found");
}

#[spec("DATA-005")]
#[tokio::test]
async fn export_blueprint_returns_pretty_printed_json() {
    let engine = DataflowEngine::new(DataflowConfig::default());
    let id = engine
        .create_pipeline_from_blueprint(filtered_blueprint("exportable"))
        .unwrap();

    let json = engine.export_blueprint(id).unwrap();

    assert!(json.contains('\n'), "export must be pretty-printed");
    let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed["name"], "exportable");
    assert_eq!(parsed["nodes"].as_array().unwrap().len(), 2);
    assert_eq!(parsed["links"][0]["rule"]["type"], "filter");
}

#[spec("DATA-005")]
#[test]
fn export_blueprint_errors_when_pipeline_not_found() {
    let engine = DataflowEngine::new(DataflowConfig::default());

    let err = engine.export_blueprint(uuid::Uuid::new_v4()).unwrap_err();

    assert_eq!(err.to_string(), "Pipeline not found");
}

#[spec("DATA-006")]
#[tokio::test]
async fn import_blueprint_round_trips_exported_json_to_new_pipeline() {
    let source_engine = DataflowEngine::new(DataflowConfig::default());
    let original_id = source_engine
        .create_pipeline_from_blueprint(filtered_blueprint("roundtrip"))
        .unwrap();
    let json = source_engine.export_blueprint(original_id).unwrap();

    let target_engine = DataflowEngine::new(DataflowConfig::default());
    let imported_id = target_engine.import_blueprint(&json).unwrap();

    assert_ne!(imported_id, original_id);
    let pipeline = target_engine.get_pipeline(imported_id).unwrap();
    let blueprint = pipeline.to_blueprint();
    assert_eq!(blueprint.name.as_deref(), Some("roundtrip"));
    assert_eq!(blueprint.nodes.len(), 2);
    assert!(blueprint.links[0].rule.is_some());
}

#[spec("DATA-006")]
#[test]
fn import_blueprint_rejects_invalid_json_with_validation_details() {
    let engine = DataflowEngine::new(DataflowConfig::default());

    let dangling_link = serde_json::json!({
        "version": "1.0.0",
        "name": "broken",
        "description": null,
        "nodes": [{"id": "input", "kind": "source", "params": {}}],
        "links": [{"from": "input", "to": "missing"}],
        "metadata": null
    });
    let err = engine
        .import_blueprint(&dangling_link.to_string())
        .unwrap_err();
    let message = err.to_string();
    assert!(
        message.contains("Blueprint validation failed"),
        "expected validation details, got: {message}"
    );
    assert!(
        message.contains("missing"),
        "expected the offending node id in the error, got: {message}"
    );

    let err = engine.import_blueprint("not json at all").unwrap_err();
    assert!(!err.to_string().is_empty());
}
