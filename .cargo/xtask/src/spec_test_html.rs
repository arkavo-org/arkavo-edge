use crate::spec_test::CoverageReport;
use crate::spec_test_export;
use anyhow::Result;
use std::path::Path;

const CSS: &str = include_str!("../templates/traceability.css");
const LAYOUT_JS: &str = include_str!("../templates/treemap_layout.js");
const TRACE_JS: &str = include_str!("../templates/traceability_standalone.js");

pub fn export_html(report: &CoverageReport, output: &Path) -> Result<()> {
    let json = spec_test_export::export_json_string(report)?;

    let html = format!(
        r#"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="UTF-8">
<meta name="viewport" content="width=device-width, initial-scale=1.0">
<title>Spec-to-Code Traceability</title>
<style>
{CSS}
</style>
</head>
<body>
<h1>Spec-to-Code Traceability</h1>
<div id="traceability-container"></div>
<script>
var DATA = {json};
</script>
<script>
{LAYOUT_JS}
</script>
<script>
{TRACE_JS}
</script>
<script>
document.addEventListener('DOMContentLoaded', function() {{
    renderTraceabilityContent(document.getElementById('traceability-container'), DATA);
}});
</script>
</body>
</html>"#
    );

    if let Some(parent) = output.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(output, html)?;
    Ok(())
}
