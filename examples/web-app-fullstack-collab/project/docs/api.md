# API Documentation Template

## Purpose
- Keep this file updated with the current REST surface and JSON schemas
- Backend agent owns accuracy, frontend agent reviews for client impacts

## Required Sections
- Overview of launch management endpoints grouped by resource
- Authentication and rate limiting expectations
- Schema definitions for request and response bodies
- Error payload format including validation failure representation
- Change log describing latest revisions with timestamps and author agent

## Update Workflow
- After backend changes, send A2A message to frontend agent summarizing modifications
- Regenerate OpenAPI spec and paste key changes here for quick reference
- Link to related regression tests covering new or changed behavior
