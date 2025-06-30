use serde_json::json;
use std::sync::{Arc, RwLock};

use crate::App;

/// Simple UI state inspector for debugging
pub struct UiInspector {
    app_state: Arc<RwLock<App>>,
}

impl UiInspector {
    pub fn new(app_state: Arc<RwLock<App>>) -> Self {
        Self { app_state }
    }

    /// Get current UI state as JSON
    pub fn get_state(&self) -> serde_json::Value {
        let app = match self.app_state.read() {
            Ok(app) => app,
            Err(_) => return json!({"error": "Failed to acquire read lock"}),
        };

        json!({
            "view_mode": format!("{:?}", app.view_mode),
            "layout_mode": format!("{:?}", app.layout_mode),
            "focused_pane": format!("{:?}", app.focused_pane),
            "input_focused": app.input_focused,
            "input_buffer": &app.input_buffer,
            "vim_enabled": app.vim_enabled,
            "active_model": &app.active_model,
            "selected_model": app.selected_model,
            "available_models": &app.available_models,
            "task_count": app.task_manager.tasks.len(),
            "active_task": app.task_manager.active_task,
            "tasks": app.task_manager.tasks.iter().map(|t| {
                json!({
                    "id": t.id.to_string(),
                    "agent_name": &t.agent_name,
                    "model_name": &t.model_name,
                    "status": format!("{:?}", t.status),
                    "message_count": t.messages.len(),
                })
            }).collect::<Vec<_>>(),
            "providers": app.providers.iter().map(|p| {
                json!({
                    "name": &p.name,
                    "type": format!("{:?}", p.provider_type),
                    "url": &p.url,
                    "status": format!("{:?}", p.status),
                    "model_count": p.models.len(),
                })
            }).collect::<Vec<_>>(),
            "server_urls": &app.server_urls,
            "configuration_mode": format!("{:?}", app.configuration_mode),
            "debug_logs": app.debug_view.get_all_logs().len(),
            "telemetry": {
                "messages_sent": app.telemetry.messages_sent,
                "messages_received": app.telemetry.messages_received,
                "key_events": app.telemetry.key_events,
            },
        })
    }

    /// Get debug logs
    pub fn get_debug_logs(&self) -> serde_json::Value {
        let app = match self.app_state.read() {
            Ok(app) => app,
            Err(_) => return json!({"error": "Failed to acquire read lock"}),
        };

        json!({
            "logs": app.debug_view.get_all_logs(),
            "count": app.debug_view.get_all_logs().len()
        })
    }

    /// Get provider status
    pub fn get_provider_status(&self) -> serde_json::Value {
        let app = match self.app_state.read() {
            Ok(app) => app,
            Err(_) => return json!({"error": "Failed to acquire read lock"}),
        };

        json!({
            "providers": app.providers.iter().map(|p| {
                json!({
                    "name": &p.name,
                    "type": format!("{:?}", p.provider_type),
                    "url": &p.url,
                    "status": format!("{:?}", p.status),
                    "models": &p.models,
                })
            }).collect::<Vec<_>>(),
            "server_urls": &app.server_urls,
        })
    }
}