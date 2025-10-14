use serde_json::json;

pub struct ComponentTemplates;

impl ComponentTemplates {
    pub fn chart_component() -> serde_json::Value {
        json!({
            "type": "chart",
            "html": r#"<div class="chart-wrapper">
    <canvas id="dynamicChart"></canvas>
</div>"#,
            "css": r#".chart-wrapper {
    width: 100%;
    max-width: 800px;
    height: 400px;
    background: var(--bg-secondary);
    padding: 20px;
    border-radius: 8px;
    border: 1px solid var(--border-color);
}"#,
            "javascript": r#"// Chart rendering logic here
const ctx = document.getElementById('dynamicChart').getContext('2d');
// Initialize chart with provided data"#
        })
    }

    pub fn metrics_table() -> serde_json::Value {
        json!({
            "type": "table",
            "html": r#"<div class="metrics-table-container">
    <table class="metrics-table">
        <thead>
            <tr>
                <th>Metric</th>
                <th>Value</th>
                <th>Status</th>
            </tr>
        </thead>
        <tbody id="metricsTableBody">
        </tbody>
    </table>
</div>"#,
            "css": r#".metrics-table-container {
    overflow-x: auto;
}

.metrics-table {
    width: 100%;
    border-collapse: collapse;
    background: var(--bg-secondary);
}

.metrics-table th,
.metrics-table td {
    padding: 12px;
    text-align: left;
    border-bottom: 1px solid var(--border-color);
}

.metrics-table th {
    background: var(--bg-tertiary);
    font-weight: 600;
}"#,
            "javascript": r#"function updateMetricsTable(metrics) {
    const tbody = document.getElementById('metricsTableBody');
    tbody.innerHTML = metrics.map(m => `
        <tr>
            <td>${m.name}</td>
            <td>${m.value}</td>
            <td><span class="status-badge status-${m.status}">${m.status}</span></td>
        </tr>
    `).join('');
}"#
        })
    }

    pub fn control_panel() -> serde_json::Value {
        json!({
            "type": "control_panel",
            "html": r#"<div class="control-panel">
    <div class="control-group">
        <label for="agentSelect">Select Agent:</label>
        <select id="agentSelect" class="control-select">
            <option value="">Choose agent...</option>
        </select>
    </div>
    <div class="control-group">
        <button id="actionBtn" class="control-button">Execute</button>
    </div>
</div>"#,
            "css": r#".control-panel {
    background: var(--bg-secondary);
    padding: 20px;
    border-radius: 8px;
    border: 1px solid var(--border-color);
}

.control-group {
    margin-bottom: 16px;
}

.control-group label {
    display: block;
    margin-bottom: 8px;
    font-weight: 500;
}

.control-select,
.control-button {
    width: 100%;
    padding: 10px;
    background: var(--bg-tertiary);
    border: 1px solid var(--border-color);
    color: var(--text-primary);
    border-radius: 4px;
    cursor: pointer;
}

.control-button:hover {
    background: var(--accent-color);
}"#,
            "javascript": r#"document.getElementById('actionBtn').addEventListener('click', () => {
    const selected = document.getElementById('agentSelect').value;
    if (selected) {
        console.log('Executing action for agent:', selected);
    }
});"#
        })
    }

    pub fn get_template_by_type(component_type: &str) -> Option<serde_json::Value> {
        match component_type.to_lowercase().as_str() {
            "chart" => Some(Self::chart_component()),
            "table" => Some(Self::metrics_table()),
            "control" | "panel" => Some(Self::control_panel()),
            _ => None,
        }
    }
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)]
mod tests {
    use super::*;

    #[test]
    fn test_template_retrieval() {
        let chart = ComponentTemplates::get_template_by_type("chart");
        assert!(chart.is_some());
        assert_eq!(chart.unwrap()["type"], "chart");

        let table = ComponentTemplates::get_template_by_type("table");
        assert!(table.is_some());

        let unknown = ComponentTemplates::get_template_by_type("unknown");
        assert!(unknown.is_none());
    }
}
