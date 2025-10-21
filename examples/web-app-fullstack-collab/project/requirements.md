# Fullstack Web App Collaboration Requirements

## Overview
- Build a production-ready launch management portal for internal feature rollouts
- Deliver a minimal slice that demonstrates backend API quality and frontend polish
- Keep the repo deterministic so automated monitors can run tests and lint checks without manual setup

## Core User Stories
- Product owners can create feature launch records with name, status, rollout date, and notes
- Engineers can attach deployment artifacts and view the audit history of a launch
- Stakeholders see a responsive dashboard with filters for status, owner, and target platform

## Service Responsibilities
- Backend owns data validation, persistence, audit logging, and REST API ergonomics
- Frontend owns user workflows, accessibility, and visual consistency with Arkavo branding
- Both agents share responsibility for end-to-end flows, CI health, and doc updates when contracts evolve

## Technical Guardrails
- API must expose OpenAPI schema under `/spec/openapi.json`
- Database layer uses SQLite with safe migrations and rollback paths
- Frontend implemented with React + Vite, ships type-safe API client generated from OpenAPI
- Reuse Arkavo logging and tracing conventions; do not add new logger dependencies

## Known Bugs To Fix
- Editing a launch record drops the previously attached deployment links
- Frontend date picker allows selecting past dates even when the launch is already complete
- Backend returns HTTP 200 when validation fails instead of 422 with error details

## Enhancements To Deliver
- Add aggregated metrics endpoint `/metrics/launch-health` that surfaces healthy, paused, blocked counts
- Provide frontend toast notifications when save operations succeed or fail
- Implement background task that archives launches older than 180 days and expose the results via `/maintenance/archive/report`

## Quality Checklist
- `cargo test -p launch-backend` and `npm test` under frontend workspace must pass
- `cargo clippy -- -D warnings` and `npm run lint` must be clean
- Include regression tests for the three known bugs after fixes ship
- Document API surface in `project/docs/api.md` and frontend workflows in `project/docs/frontend.md`

## Collaboration Rules
- Agents exchange updates through A2A `message/send` with structured payloads
- Use the monitoring script to trigger verification runs before closing tasks
- The first agent to merge changes notifies the other using `agent_query` status updates
- Both agents update the shared docs when data contracts or UI flows change
