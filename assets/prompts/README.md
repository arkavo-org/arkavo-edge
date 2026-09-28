Prompts README

Purpose
- This directory contains prompt specifications used by humans and AI agents to consistently produce artifacts (docs, configs, code, etc.).
- Goals: clarity, repeatability, and maintainability.

Naming Conventions
- General pattern: [output_file].[type].[format]
  - output_file: the final artifact this prompt produces (e.g., agents_md → AGENTS.MD).
  - type: what this file is (prompt).
  - format: the prompt’s authoring format (md, json, etc.).
- Example (recommended, concise):
  - chat_system.prompt.md
  - Why it’s good: Within assets/prompts/, it’s clear this prompt generates the chat system prompt. It’s short and follows [output_file].[type].[format].
- Guidance:
  - Choose descriptive, consistent names aligned with nearby prompts (e.g., system_prompt).
  - Prefer lowercase with underscores for multiword names.
  - Keep names stable; renaming prompts creates churn in tooling and docs.

Recommended Prompt Structure
Include these sections in each .prompt.md (adapt as needed):
1) Title and Version
   - Example: Title: Generate AGENTS.MD (v1.2)
2) Goal
   - One to three sentences that define the outcome and audience.
3) Inputs and Variables
   - List required inputs and placeholders, e.g., {{repo_name}}, {{contributors}}.
4) Output Format
   - Exact schema or file structure; specify allowed sections and ordering.
5) Constraints and Style
   - Tone, length limits, formatting rules, forbidden content or behaviors.
6) Process / Steps
   - High-level steps the agent should follow to produce the output.
7) Examples (Few-shot, optional)
   - 1–3 illustrative input → output snippets that demonstrate the style.
8) Quality Checklist
   - Acceptance criteria the agent must self-verify before finalizing.
9) Notes and Edge Cases
   - Known pitfalls, domain-specific rules, and disambiguation guidance.

Authoring Guidelines (for humans)
- Be explicit about audience and success criteria; avoid ambiguous verbs (e.g., “summarize how?” → “summarize in ≤200 words, bullet points, no jargon”).
- Keep scope tight; split complex tasks into separate prompts or stages.
- Specify an exact output schema (sections, headings, fields) and disallow extra commentary.
- Use variables with a consistent format (e.g., {{variable_name}}) and document each in Inputs.
- Provide at least one example when style or structure isn’t obvious.
- Add a Quality Checklist that is objective and verifiable.
- Version your prompts (e.g., v1.2) and note changes at the bottom.
- Include constraints for length, tone, formatting, and references if required.
- List prohibited behaviors (e.g., “Do not invent data; use only provided inputs.”).

Execution Guidelines (for AI/agents)
- Read the entire prompt; do not skip sections.
- If required Variables are missing, request them before proceeding.
- Follow the Output Format exactly; do not add extra sections or commentary.
- Use the Process / Steps as the plan; do not reorder if the prompt forbids it.
- Validate against the Quality Checklist before producing the final output.
- When ambiguity remains, state assumptions clearly and proceed conservatively.
- Prefer deterministic, reproducible results; avoid randomness unless instructed.

Versioning and Maintenance
- Add a version at the top (e.g., v1.0, v1.1) and a brief Change Log at the bottom.
- When making breaking changes to structure or semantics, increment the minor/major version and notify downstream users.
- Deprecate old prompts explicitly if they’re replaced; keep a note referencing the new prompt.

File Organization
- Keep prompt files in assets/prompts/.
- Use one prompt per outcome; create separate prompts for clearly different artifacts/workflows.
- Co-locate related prompts with consistent naming, following the [output_file].[type].[format] pattern.

Minimal Template (copy/paste)

# Title: Generate {{output_file}} (v1.0)

## Goal
Briefly describe the outcome, audience, and definition of done.

## Inputs and Variables
- {{variable_1}}: description
- {{variable_2}}: description

## Output Format
- Produce exactly the following sections in order:
  1. Heading: ...
  2. Subsections: ...
- Formatting rules:
  - Markdown only, no HTML.
  - No extra commentary outside specified sections.

## Constraints and Style
- Tone: professional, concise.
- Length: ≤ N words per section.
- Citations: if any, use [label](url) format.
- Forbidden: hallucinations, ungrounded claims, extra sections.

## Process / Steps
1. Validate inputs.
2. Extract key points from {{variable_1}}.
3. Organize content per Output Format.
4. Run Quality Checklist and fix issues.

## Examples
Input (excerpt):
...
Output (excerpt):
...

## Quality Checklist
- [ ] All required sections present and in correct order.
- [ ] No extra sections or commentary.
- [ ] Style and length constraints satisfied.
- [ ] Variables fully resolved (no {{...}} remain).
- [ ] Links and references, if any, are valid.

## Notes and Edge Cases
- If {{variable_2}} is empty, omit section X.

## Change Log
- v1.0: Initial version.

Example Names
- chat_system.prompt.md → Generates the chat system prompt.
- changelog_md.prompt.md → Generates CHANGELOG.md entries.
- release_notes_md.prompt.md → Generates release notes in Markdown.

Quick Rationale Recap
- The name you choose for each prompt is important for clarity and maintainability.
- Within assets/prompts/ alongside other configuration prompts (like system_prompt), use descriptive, consistent names.
- Simpler, also good option: chat_system.prompt.md
  - Why it’s good: Short, clear, and follows [output_file].[type].[format]; within assets/prompts/, it’s understood that the prompt is for generating the chat system prompt.