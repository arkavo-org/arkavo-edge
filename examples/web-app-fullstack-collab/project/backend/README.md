# Backend Game Plan

## Initial Deliverables
- Scaffold a Rust axum service with routes under `/launches` and `/maintenance`
- Implement SQLite migrations stored in `migrations/` with deterministic ordering
- Generate OpenAPI spec using utoipa and expose it via `GET /spec/openapi.json`

## Bug Fix Targets
- Ensure `PUT /launches/{id}` preserves `deployment_links` field when omitted in payload by loading existing data first
- Return HTTP 422 with a JSON error payload for validation failures during create/update
- Add unit tests covering the regression scenarios in `src/tests/regressions.rs`

## Enhancements
- `/metrics/launch-health` aggregates totals grouped by status with cached computation to keep under 25ms
- Archive job runs hourly and persists reports in `archive_reports` table with job metadata
- Provide SSE endpoint `/events/launches` for frontend live updates once base APIs stabilize

## Monitoring Hooks
- Expose `/health` with database connectivity check and migration drift detection
- Publish structured logs to stdout; monitoring script tails `logs/backend-agent.log`
- Include tracing spans named `launch_backend::*` for key operations

## Collaboration Expectations
- Emit A2A status messages containing `{"kind":"backend-update","status":"..."}` when schema changes
- Announce API breaking changes ahead of merges and coordinate client updates with frontend agent
- When tests fail locally, push diagnostics to `project/logs/backend` for triage by monitoring script
