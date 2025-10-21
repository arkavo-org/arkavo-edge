# Frontend Game Plan

## Initial Deliverables
- Bootstrap a Vite + React + TypeScript app inside `project/frontend/app`
- Build dashboard view listing launches with filters for status, owner, and platform
- Implement launch detail drawer with edit form wired to backend validation responses

## Bug Fix Targets
- Preserve deployment links when editing by diffing form state against the loaded record
- Block selecting past dates for completed launches using a shared validation helper
- Surface backend validation errors inline next to the relevant fields and show toast summary

## Enhancements
- Add health summary panel that calls `/metrics/launch-health`
- Show archive notifications using `/maintenance/archive/report`
- Stream live updates with EventSource bound to `/events/launches`

## Monitoring Hooks
- Expose `npm run lint`, `npm run test`, and `npm run typecheck` scripts for monitoring automation
- Write end-to-end smoke test in `tests/smoke.spec.ts` using Playwright for regression coverage
- Emit telemetry breadcrumbs to console with `window.__arkavoMonitoring?.push(...)` for debugging

## Collaboration Expectations
- Notify backend agent via A2A `message/send` when client contracts change
- Pull latest OpenAPI before regenerating API client and attach diff summary in status updates
- Store shared UI copy in `project/docs/frontend.md` so both agents keep messaging consistent
