# Arkavo Operations: web UI redesign blueprint

Review date: 2026-10-17. Baseline: workspace version 0.105.0.

## Product decision

Recreate `arkavo ui` as **Arkavo Operations**, an operations console for directing agents, understanding their authority, and intervening with evidence. The home screen should answer: **What is happening, what needs my decision, what can continue, and what do we actually know?**

The product opportunity is the combination of local inference, distributed coordination, explicit execution policy, protected data exchange, and constrained adaptation. The interface should make that combination inspectable through everyday work. Decorative network graphs and generic chat are secondary to mission progress, decisions, and the limits of available evidence.

The governing UI principle is **maximum useful information with minimum interpretation**. Optimize for recognizing a state and choosing the next action. Use a shared lexicon, fixed field positions and short labels. Put explanations behind a circled information icon; show supporting records on selection. More data is useful only when the operator can distinguish its meaning, age and scope without reconstructing them from prose.

For this design, “contested” means unreliable or denied connectivity, constrained compute, untrusted peers and inputs, and potentially compromised workers. The scope is defensive operations on authorized systems: investigation, integrity checks, incident triage, controlled remediation, continuity, and recovery. The design does not assume that a compromised host administrator can be contained by a process on that same host.

This is a design blueprint, not an implementation or a declaration of operational security readiness. Proposed controls below require backend enforcement before they become available.

## Review basis and confidence

Reviewed the specification corpus structure, the relevant UI, orchestration, routing, budget, identity, ingress, policy, data-protection and attestation scenarios, and their principal UI/backend paths. A source inventory found 91 component spec files, 978 scenario-ID entries and 105 `wip: true` markers. These are inventory counts, not test results or coverage measurements. `specs/README.md`, `specs/FUTURE.md`, and the aggregate index contain older counts; use component scenarios and implementation evidence when deciding readiness.

The browser review used the repository's actual HTML/CSS/JavaScript served locally, including a Chrome walkthrough of navigation and the task composer. No live gateway or agent swarm was attached. The visible disconnected state is therefore useful evidence about presentation and interaction, not evidence about live agent execution. No inference, containment, approval or task dispatch was exercised against a running agent.

The accompanying design concept was checked in Chrome at desktop and 320 px widths. The lexicon revision was checked at 1024 px and 320 px, including record inspection, connected/link-loss/containment scenarios and keyboard access to information help. Local source links and JavaScript syntax were checked. The concept uses illustrative records and makes no network calls; it is not wired into the production UI. Touch behavior and a full accessibility audit remain implementation validation work.

Readiness terms used here:

| Label | Meaning |
|---|---|
| Existing UI path | The browser handler and corresponding backend path were inspected; live behavior still needs integration validation. |
| Runtime capability | Relevant implementation/spec exists, but the required operator workflow is absent or incomplete. |
| Proposed | Design target or `wip` behavior; do not represent it as an enabled defense. |

## Current functionality to preserve

The browser client lives in `crates/arkavo-agui/static/`, with embedded static assets served by the Rust gateway. It uses `/ws`; a separate `/api/agent` SSE endpoint implements canonical AG-UI streaming. The CEF renderer is a distinct path selected by build features and is outside this first web redesign.

| Current surface | Existing functionality | Destination in the redesign |
|---|---|---|
| Agent Mesh | mDNS discovery, agent purpose/model/endpoint, connection status, per-agent streaming chat | Swarm; contextual conversation in mission detail |
| Tasks | Free-text submission, task list, status/progress, result, routing and A2A/telemetry detail | Missions |
| Budget & ROI | Spending, cost metrics, budget alerts, ROI, compute/system metrics | Runtime → Resources; relevant limits beside mission actions |
| Health | Component health, model availability, uptime, memory, tool availability, timing | Runtime → Health; exceptions on Overview |
| Security & Data Plane | Per-agent KAS/Iroh state, preflight status, TDF audit, policy events, transfers, published trust | Data & Trust; related records in Evidence |
| Router | Model decisions and routing context | Runtime → Routing; decision excerpt in mission detail |
| Telemetry | Bounded event stream and performance summaries | Evidence; Runtime → Performance |
| Learning | Routing evaluations/outcomes, priors, lessons, quality trends | Runtime → Adaptation |
| Context Topology | Per-agent context pressure, memory, anti-patterns, traces and gossip | Runtime → Context |
| Agent Runtime Policy | Per-agent documents, proposal review, violations, policy cache, adaptation, traces, SwarmFlight roles and launch errors | Decisions; Missions → Flights; Data & Trust → Policy |
| Showcase | Automatic view switching | Optional presentation mode outside the normal operations workflow |

Sources: [browser shell](../crates/arkavo-agui/static/index.html), [event routing](../crates/arkavo-agui/static/js/app.js), [gateway static routes](../crates/arkavo-agui/src/gateway_static.rs), [AG-UI scenarios](../specs/arkavo-edge/agui.spec.yaml).

## Findings that change the design

| Priority | Finding | Design consequence |
|---|---|---|
| Critical | Gateway connection, peer discovery, KAS availability and authorization are different facts. Current surfaces do not give them a consistent evidence/freshness model. | Show each independently. Never compute a universal “secure” score from them. |
| Critical | The task composer remains usable while disconnected. `wsSend` silently does nothing if the socket is not open, while `submitTask` clears and closes the composer. | Preserve the draft; require an acknowledged submission; show “not sent” or “outcome unknown.” Never automatically resend uncertain commands. |
| Critical | `requestStopFlight` removes registry/ARP entries; the inspected path does not cancel remote work, revoke credentials or enforce containment. | Label its actual effect as ending gateway flight tracking. It must not power a “contain” or emergency-stop button. |
| Critical | Proposal review records the fixed reviewer string `agui-operator`. Browser origin/Host defenses are not individual operator authentication. | Attributable approval needs an authenticated operator and scoped authorization, not an identity field typed in the browser. |
| Critical | Ingress membership, operator socket, full command confinement and several egress/key-lifecycle defenses are marked incomplete. | Show capability coverage and unknown states. Do not claim the deployed swarm is ready for hostile networks from UI appearance. |
| High | Ten peer-level technical panels force operators to correlate tasks, policy, costs and findings mentally. | Organize around missions and decisions; reuse the same agent/task/policy identity across all detail views. |
| High | A task description is presented as submission to an orchestrator, but the gateway selects an agent and delegates; decomposition depends on that agent. | Describe the actual dispatch route. Show an execution plan only when a planner has supplied one. |
| High | Gateway task tracking is an in-memory map. Persistence promised by the task orchestration subsystem does not establish persistence of this UI's task history. | Define durable mission history and reconciliation explicitly; do not imply refresh/restart recovery that the gateway cannot prove. |
| High | Security/data-plane views substitute zeros or booleans where missing information can be ambiguous. | Distinguish zero, unknown, unavailable, stale, denied and unsupported. Give each a next step. |
| High | Important approval failures currently go to the console; the acting operator may not see why a request failed. | Keep action results in the relevant record, with retry rules and an audit reference. Toasts supplement persistent feedback. |
| Medium | Icon-only navigation, scattered empty states, and limited dialog semantics increase learning and keyboard costs. | Visible labels, clear grouping, native controls, focus management and plain-language empty states. |
| Medium | Some metrics are estimates, including task-energy calculations with fixed power/price defaults. | Label measured, estimated and unavailable values; display assumptions beside derived metrics. |
| Medium | Showcase can switch the operator's view during activity. | Never automatically navigate away from an active decision or investigation. |

Evidence: [task client](../crates/arkavo-agui/static/js/panels/tasks.js), [WebSocket client](../crates/arkavo-agui/static/js/websocket.js), [dispatch](../crates/arkavo-agui/src/gateway_task_dispatch.rs), [gateway state](../crates/arkavo-agui/src/gateway.rs), [review/stop handlers](../crates/arkavo-agui/src/gateway_ws.rs), [flight deregistration](../crates/arkavo-agui/src/swarm_flight_registry.rs), [security view](../crates/arkavo-agui/static/js/panels/security.js), [task metrics](../crates/arkavo-agui/static/js/panels/task-detail.js).

## Market position

Recommended positioning: **“Operate AI agents at the edge, with visible boundaries on what they can do and where data can go.”**

This is a positioning hypothesis grounded in the repository, not a claim of exclusive capability or measured superiority. The initial customer wedge is teams operating small or distributed agent swarms that need local execution, constrained authority, and clear operator intervention. Validate deployment effort, hardware requirements and integration cost with those teams before claiming lower total cost.

### What is already common in the market

Multi-agent coordination, human approval, tracing and durable workflows are established capabilities. LangGraph documents persistence, streaming and human-in-the-loop orchestration; Microsoft's Agent Framework documents sequential, concurrent, handoff, group-chat and manager-led orchestration with tool approvals. These are necessary product capabilities, not strong standalone differentiators. [LangGraph overview](https://docs.langchain.com/oss/python/langgraph/overview), [Microsoft orchestration documentation](https://learn.microsoft.com/en-us/agent-framework/workflows/orchestrations/).

Disconnected edge operations are also an established category. Palantir documents disconnected environments and separate connected, intermittent and relayed operating modes. Arkavo should compete on its specific deployable runtime and policy/data model, not claim to have invented offline operations. [Apollo environments](https://www.palantir.com/docs/apollo/core/environments), [environment connection settings](https://www.palantir.com/docs/apollo/managing-environments/environment-connection-settings).

### Differentiators to make tangible

| Candidate differentiator | Repository support and boundary | UI proof point |
|---|---|---|
| Local inference with deliberate cloud escalation | Router local-only behavior, feasibility/quality routing and cloud-spend authorization are specified and implemented in relevant paths. Local model availability does not prove end-to-end offline mission readiness. | Show execution locality, resident model readiness, cloud permission and the reason for each escalation. |
| Authority tied to an invocation | Permit-bound MCP proxy dispatch verifies issuer, proof of possession, tool/arguments, time and invocation budget. Coverage is the enforced proxy path, not every possible tool/process/network route. | Action receipt shows subject, tool, argument digest, destination when available, permit reference and admission/denial. Show mediated versus uncovered paths. |
| Policy travels with protected data | TDF wrapping, ABAC/KAS and Iroh sharing exist. Several revocation, key lifecycle and broader data-security scenarios remain WIP. Encryption does not retract plaintext already disclosed. | Follow a protected artifact from source policy through key release and transfer; expose recipient and evidence limitations. |
| Adaptation with constrained effects | ARP tightening proposals have blast-radius checks, explicit lifecycle states, displaced values and reversal paths. Mesh-wide changes cannot auto-apply; capability expansion has separate gates. | Review exact before/after policy, affected scope, evidence and observation outcome; distinguish tightening from privilege expansion. |
| Portable role packages and distributed execution | SwarmKit validation, role policy/runtime, role binding and flight visibility exist. A logical flight and a remote execution swarm are distinct objects. | Inspect kit digest, role-to-agent binding, effective policy, launch failures and per-role results. |
| Evidence-led defensive control | Trust-state, Guardian, freshness and containment architecture is documented, with substantial integration still proposed. | A future control surface that shows who authorized an action, which boundary enforced it, and which nodes have acknowledged containment. |

Support: [router](../specs/arkavo-edge/router.spec.yaml), [budget](../specs/arkavo-edge/budget.spec.yaml), [permit dispatch](../specs/arkavo-edge/permit-dispatch.spec.yaml), [TDF](../specs/arkavo-edge/tdf.spec.yaml), [TDF security](../specs/arkavo-edge/tdf-security.spec.yaml), [ARP](../specs/arkavo-edge/arp.spec.yaml), [SwarmKit](../specs/arkavo-edge/swarmkit.spec.yaml), [swarm trust design](swarm-trust-model.md).

Avoid “military-grade,” “fully zero trust,” “all tools sandboxed,” “instant swarm containment,” “autonomous declassification,” “guaranteed air-gap readiness,” or unmeasured savings/latency percentages. The ≤50 ms router and ≤65 MB binary requirements are engineering targets until measured for the shipping build. The UI should highlight concrete evidence rather than promotional badges.

## Users and authority

| Operator responsibility | Primary question | Primary workspace |
|---|---|---|
| Mission owner | Is the objective progressing within its scope and limits? | Overview, Missions |
| Security reviewer | Which actions or agents need intervention, and what evidence supports it? | Decisions, Data & Trust, Evidence |
| Runtime maintainer | Which resource, model, transport or configuration is causing failure? | Swarm, Runtime |

These are workflow responsibilities, not roles already enforced by the gateway. Future grants must distinguish task dispatch, policy approval, containment, recovery and protected-evidence access. A mission owner must not gain recovery or data-release authority merely by owning the mission.

## UI lexicon

This is the canonical product vocabulary for navigation, labels, notifications, help and assistant-generated summaries. Keep backend enum values unchanged where needed; map them through a shared presentation dictionary. A new label needs a definition and a distinct operator use. Do not introduce synonyms to vary the writing.

### Objects and actions

| Term | Icon (Lucide) | Exact meaning | Usage rule |
|---|---|---|---|
| Mission | `target` | An objective, scope, constraints and success criteria grouping tasks | Use only when a mission record exists. Existing standalone work remains a Task. |
| Task | `list-todo` | One assigned unit of work | Do not alternate with job, mission or operation. |
| Run | `workflow` | One execution attempt of a task | A retry creates another attempt; it does not rename the task. |
| Agent | `bot` | An identified worker that accepts work | Use node or host only when discussing the underlying device. |
| Swarm | `network` | Agents within the selected scope | Discovery alone does not establish membership or authority. |
| Scope | `scan` | The resources and agents a view or action applies to | Keep visible beside consequential actions. |
| Policy | `scroll-text` | Rules evaluated at a named enforcement boundary | Policy is not evidence that every path is enforced. |
| Grant | `key-round` | Authority for a principal to act on a resource | Keep recipient, action and expiry inspectable. A grant is not connectivity. |
| Decision | `git-pull-request` | A request requiring an authorized human choice | Name its kind: policy change, data release or recovery. The proposal is the exact change being reviewed. |
| Event | `activity` | A recorded observation or transition | An event is not automatically proof of enforcement. |
| Receipt | `receipt-text` | A response from the component handling an action | Name its issuer and whether it confirms acceptance or enforcement. |
| Evidence | `files` | Records supporting a specific claim | Show source, age and verification separately. |
| Artifact | `file` | A file or other addressable output | Use the artifact's short name in rows; protection and provenance are fields. |
| Boundary | `fence` | A point that enforces a specified rule | Use the concrete name, such as Tool execution or Key release, in the UI. |

Keep subsystem terms such as ARP, KAS, TDF and SwarmFlight in advanced detail and searchable aliases. Default labels are Runtime policy, Key service, Protected artifact and Flight. A Swarm is the group of agents. A SwarmKit defines coordinated work; a SwarmFlight is one execution instance of that kit, not a synonym for every mission. These definitions follow the [SwarmKit draft](https://github.com/arkavo-org/specifications/blob/main/swarmkit/swarmkit-spec-draft-01.md). Expand unfamiliar abbreviations in information help, rather than repeating expansions in every row.

Use stable action labels: **Create task, Inspect, Review, Approve change, Reject change, Request containment, Request recovery**. Use **End tracking** for today's flight deregistration. Reserve **Cancel task** for an implemented cancellation contract. Avoid generic Execute, Proceed, Resolve, Stop or Approve all where the effect is ambiguous.

### State vocabulary

Each field answers one question. A row may have both an execution state and an observation age; no combined health or trust badge replaces them.

| Field | Canonical values and meaning | Never infer |
|---|---|---|
| Link | Connected; Disconnected; Reconnecting | Connected does not mean authorized or current. |
| Task | Draft; Queued; Running; Blocked; Completed; Failed; Cancelled | Blocked needs a visible reason. Cancelled requires confirmation that execution ended. A missing update cannot change the last reported task state. |
| Command | Not sent; Sending; Accepted; Rejected; Outcome unknown | Accepted means admitted, not completed. A lost reply is Outcome unknown, not Failed. |
| Review | Needs review; Approved; Rejected; Expired | Approved does not mean a policy change was applied. Track application through its receipt. |
| Authorization | Allowed; Denied; Unknown | Always bind this field to an action and scope. Service unavailability is a reason, not an authorization grant. |
| Enforcement | Requested; Enforced; Failed; Unconfirmed | Enforced requires a receipt for the named boundary. An unreachable boundary remains Unconfirmed. |
| Observation | Current; Stale; Unknown | Show age beside the value. Current requires a source-specific threshold and known clock basis. Unknown means the observation or its age cannot be established. |
| Availability | Available; Unavailable; Unsupported; Unknown | Unsupported means the capability does not exist in this deployment; Unavailable means it exists but cannot currently be used. |
| Security state | Unassessed; Eligible; Suspended; Quarantined | These are target authority-issued states. Until implemented, show Not reported; do not derive them from link state. |

These are presentation contracts, not newly implemented runtime capabilities. Render a value only when its source supports that meaning. Preserve a narrower backend state in detail if mapping would lose meaning. Mission progress derives from declared completion criteria, not an average of task percentages.

Use **Unknown** for a value that cannot be established, **Not reported** when a source has supplied no observation, **Not applicable** when a field does not apply, and **0** only for a measured zero. Avoid bare dashes and generic Pending, Healthy, Secure or Trusted. Store stable reason codes separately from short display reasons.

### Iconography

Icons are the visual form of the lexicon. Use them to recognize an object, locate a state or identify an action before reading its short label. Keep a stable mapping in the shared presentation dictionary: semantic key, label, icon, tone and information-help text. The same concept keeps its icon across navigation, rows, inspectors and notifications.

Use one outline family: **Lucide**, with 16 px icons in rows and 18 px in navigation, consistent 2 px strokes and no decorative backgrounds. Bundle only the selected icons locally in the production UI. No emoji, animated spinners for ongoing work, custom tactical symbols or runtime icon downloads. Verify names against the pinned icon package during implementation.

| Navigation | Icon | Meaning |
|---|---|---|
| Overview | `layout-dashboard` | Current operations |
| Missions | `target` | Objectives and their work |
| Swarm | `network` | Agents in scope |
| Decisions | `git-pull-request` | Requests for human review |
| Data & Trust | `shield` | Policy, identity and protected data; the plain shield names the workspace and makes no security claim |
| Evidence | `files` | Supporting records |
| Runtime | `cpu` | Execution resources and diagnostics |

Use a small set of state symbols where they improve discrimination. The field and visible state label carry the exact meaning; the icon never replaces them.

| State | Icon | Visual meaning |
|---|---|---|
| Connected / Disconnected | `plug` / `unplug` | Link present / absent |
| Running | `circle-play` | Work in progress; static, not an animation |
| Blocked | `octagon-pause` | Work cannot proceed; show its reason |
| Completed | `circle-check` | Declared work finished |
| Failed | `circle-x` | Confirmed failure |
| Needs review | `clipboard-pen-line` | Human decision needed |
| Accepted | `inbox` | Command admitted |
| Denied | `ban` | Specified action refused |
| Requested | `send` | Intervention requested |
| Enforced | `shield-check` | Named boundary confirmed the effect |
| Unconfirmed | `shield-question` | Requested boundary effect lacks confirmation |
| Unknown / Outcome unknown | `circle-help` | Answer not established |
| Stale | `history` | Observation exceeded its freshness threshold |

States without an assigned icon remain text-only. In particular, Not reported must not borrow a failure symbol. A check on Completed cannot be reused to imply authorization, healthy infrastructure or verified identity. A shield-check cannot be inferred from encryption, connectivity or a submitted request.

For actions, use `plus` with Create task, `scan-search` with Inspect, `clipboard-pen-line` with Review, `check` with Approve change, `x` with Reject change, and `shield-plus` with Request containment. Action icons are inside visibly labeled buttons; their button context distinguishes intent from a state already observed. Keep repeated row actions text-only when icons add clutter. A containment button must retain **Request** until enforcement is confirmed.

- Put object icons before names in navigation and mixed-object lists. Omit repeated object icons in a homogeneous table; its heading already establishes the type.
- Put a state icon directly before its label in the state column. Keep freshness icons beside observation age. Limit a row to one object icon when needed and one state icon, with a separate freshness marker only when stale.
- Keep object icons neutral; selection uses the accent color. State icons inherit the existing state tone, paired with visible text. Distinguish shapes in monochrome; never make color the only difference.
- Reserve the circled **i** (`info`) for explanation. It must never mean unknown, warning or failure. The question mark (`circle-help`) means unknown and is a display state, not a help button.
- Preserve visible navigation and consequential action labels. Information, Close and similarly familiar utility controls may be icon-only with accessible names and hover/focus help. Keep adjacent decorative icons `aria-hidden="true"` so screen readers announce the label once.
- Make the hit area independent of glyph size: about 44 px on touch, with spacing between targets. No hover requirement for critical facts. Keep the existing keyboard and tap behavior for circled-i help.
- If an icon cannot load, the label and action must remain fully usable. Treat successful icon rendering as presentation, never as evidence of operational success.

### Reading budget and information help

- Use one- or two-word labels where their meaning is clear. Prefer a short exact phrase over an invented acronym. Use sentence case and familiar units: `2 s`, `4 min`, `12/16 checks`, `8/12 GiB`.
- Use a stable row grammar: **Item → State → Reason or change → Age → Action**. Omit empty optional fields; never shift a column's meaning between rows. Short IDs accompany names where correlation matters.
- Show exceptions first, with comparable normal work available in the same view. Group repeated alerts by cause and affected scope; retain affected items and evidence in detail. Do not merge independent failures into a generic incident.
- Keep rows stable during inspection. Offer an explicit update when new events would reorder them. Counts are scoped and name their denominator; no percentage without a known total.
- Remove slogans, introductory paragraphs, repeated section descriptions, redundant badges and decorative metric cards from operating screens. Use alignment and spacing before adding containers or color.
- Put definitions, units, calculation assumptions and background explanations behind a small **circled i** control next to the relevant field or section heading. One shared explanation belongs at the column heading, not on every row. Use the consistent circled information icon (`info` in Lucide).
- Information help opens on hover and keyboard focus; tap or click pins it, Escape dismisses it. Keep it open while the pointer moves over the help. Place it beside the trigger without clipping. Use a native button named `About [field]` and associate the explanation for assistive technology. Touch users must not depend on hover.
- Keep help to one definition and, when necessary, one distinction. Put longer material behind a labeled detail link or disclosure. Do not put interactive links inside a tooltip; use a popover or detail view for those.
- Keep action targets, scope, blockers, stale age, missing evidence, irreversible effects and unconfirmed outcomes visible. Explanation may be optional; information necessary to make the action safely is not.
- Use progressive detail: **scan** for status and next action; **inspect** for exact scope, change and reason; **verify** for source records and receipts. Keep the selected item and list position when opening detail.
- Assistant summaries use the same terms and short fact fields. Expose rationale on demand. The model may summarize evidence but cannot generate authoritative state labels or receipts.

| Verbose presentation | Default presentation | Circled-i explanation |
|---|---|---|
| Recipient policy required before release | **Blocked · Release grant missing** | A release grant authorizes this recipient and destination. |
| The selected model route is local | **Route · Local** | Inference runs on the selected local device. Data access and sharing require separate authority. |
| Agent visibility was lost and the latest command outcome is unknown | **Link · Disconnected** / **Command · Outcome unknown** | A missing reply does not establish whether the command was accepted. |
| Containment request sent but remote enforcement has not been confirmed | **Containment · Unconfirmed** | Requested containment requires a receipt from each targeted enforcement boundary. |
| Execution scope, effective tool policy and evidence freshness are still required | **Not sent · Scope incomplete** | List the missing fields in the inspector, rather than hiding required work inside help. |

## Information architecture

Use seven labeled destinations. Advanced subsystem detail becomes a tab within the relevant workspace, rather than another primary navigation item.

| Destination | Primary content | Primary action |
|---|---|---|
| **Overview** | Mission exceptions, pending decisions, connectivity/evidence gaps, recent consequential events | Open the highest-priority actionable item |
| **Missions** | Objective, scope, steps, role bindings, results, limits, execution receipts; SwarmFlights as a subview | Create a mission or inspect one |
| **Swarm** | Agents, capabilities, model availability, reachable peers, last observation, identity and assurance separately | Inspect an agent and its assigned work |
| **Decisions** | Policy proposals, approval requests and recovery requests, separated by kind and authority | Review a concrete change |
| **Data & Trust** | Effective policy, identity evidence, grants, KAS/key release, protected artifacts and transfers | Explain an allow, deny or inability to decide |
| **Evidence** | Correlated task, policy, tool and transfer timeline; event gaps and provenance | Reconstruct an outcome or prepare an authorized evidence export |
| **Runtime** | Health, resources/budget, routing, context, adaptation and transport diagnostics | Resolve a runtime constraint |

Keep the same mission, agent, flight, proposal and incident identifiers in every workspace. Deep links restore scope and selected record. Browser back should return to the previous investigation state.

### Persistent operational context

The top bar shows the selected scope and separate states for browser→gateway, gateway→agents, and policy/security services where those services actually report state. The page also shows observation age and collection gaps. Internet availability is not a proxy for local mesh health; WebSocket connectivity is not a proxy for authorization.

Use the separate state fields in the UI lexicon. A compact line can read `Gateway · Connected` and `Swarm · 3/4 reporting`; it cannot compress them into “System healthy.” For authorization, pair `Allowed` with the action and scope. Source and verification level must be inspectable. A self-report is labeled as such. Definitions belong behind the adjacent circled i.

Classification markings come from authoritative metadata at the relevant scope. Never add a decorative “UNCLASSIFIED” or “SECRET” banner, infer a clearance from a display name, or permit free-text editing of classification as an access grant.

### Overview layout

```text
┌─────────────────────────────────────────────────────────────────────┐
│ ARKAVO / Operations     Scope                 Operator / authority   │
├─────────────────────────────────────────────────────────────────────┤
│ Gateway: Connected │ Swarm: 3/4 reporting │ Authority: Unknown  [A] │
├──────────────┬──────────────────────────────────┬───────────────────┤
│ Overview     │ Needs attention: 2            [B]│ Inspector         │
│ Missions     │ Item / State / Reason / Age      │ Scope / change    │
│ Swarm        │                          Review  │ Source / age   [C]│
│ Decisions    ├──────────────────────────────────┤                   │
│ Data & Trust │ Missions                      [D]│ Explanation: ⓘ    │
│ Evidence     │ Name / State / Next / Age        │ Records: open  [E]│
│ Runtime      │                                  │                   │
│              ├──────────────────────────────────┴───────────────────┤
│              │ Boundaries / Recent changes / Collection gaps    [F]│
└──────────────┴──────────────────────────────────────────────────────┘
```

- **A — Context:** independent status values, scoped counts and observation ages; definitions behind circled i controls.
- **B — Attention:** blocked work and pending decisions share a compact queue. Rank by explicit consequence, then deadline, then age, with stable ID as a tie-breaker. Keep consequence and deadline sources in detail; never invent severity from model prose.
- **C — Inspection:** exact scope, before/after change, source and expiry. Review opens detail; approval never occurs from a summary row.
- **D — Work:** aligned names, states, next steps and ages. Show progress only when its denominator is known.
- **E — Detail:** open supporting records without losing selection or list position. Detail becomes a full page on small screens.
- **F — Changes:** exceptions in operating boundaries and recent consequential events; raw telemetry lives in Evidence.

Do not fill the overview with lifetime event counts, arbitrary trust scores or unverified “threat levels.” No map until real location data and a location-dependent operator task justify one. A dependency graph is more useful than decorative geography for this product.

## Key workflows

### Create and run a mission

Start with a compact brief: objective, authorized scope/assets and success evidence. Resolve available agents, tools, models and default policy automatically. Show the effective execution locality, tool/data permissions, budget ceilings and stop conditions before dispatch. Ask only for information the environment cannot resolve safely; a prompt sentence is not an enforceable permission.

The first implementation can preserve today's free-text task submission while clearly calling it a task. A richer “mission” requires a durable parent record, child tasks, constraints and result criteria. Do not manufacture a plan or display an unsupported mission lifecycle around a single task.

Use the lifecycle terms in the UI lexicon: Draft → Queued → Running → Completed, with Blocked, Failed and Cancelled only when the backend supports those meanings. Validation is a submission step, not a fabricated execution state. Drafts remain unsent until admitted. Dispatch shows an operation ID, followed by the authoritative task ID on acknowledgment. A dropped reply produces **Outcome unknown**, not success or automatic replay. Recovery queries by the same idempotency key once durable reconciliation exists; earlier phases retain the uncertain outcome and disable blind retry.

Mission detail combines objective, current step, agent/role, permitted actions, resource use, result artifacts and evidence. A contextual assistant may explain or propose a next step; applying a change uses the same typed, authorized command path as the visible UI. Model reasoning text is not an audit record or policy decision.

### Review a proposed change

Select an item in Decisions to see its kind, submitting principal, target agent/flight, policy version, exact before/after effect, rationale, evidence references, blast radius, expiry and observation/reversal behavior.

Keep distinct queues for tightening proposals, capability expansion, protected-data release and recovery. They have different authorities and cannot share a generic “Approve all” operation. Today's ARP approve/reject path is the first supported decision type; the other queues remain absent until backed by real capabilities.

Approval binds the reviewer, target, proposal digest and policy version. The server revalidates current authority and the change against current state. A concurrent edit, expiry, disconnect or policy-generation change invalidates the approval context and returns the record for review. Display Applied only after the server confirms the effect, then display observation results and any reversal. Capture a reason where policy requires it.

### Investigate and contain suspected compromise

Open an incident with authenticated evidence, affected workload/session, impacted missions and provenance of derived artifacts. A routine policy refusal is not automatically a compromise. Keep Detection, Assessment, Containment request and Enforced containment as separate facts.

The proposed action sheet names the exact scope and expected effects: block new task admission, tool execution, model egress, key release, delegation and publication for that scope where enforcement exists. Surface uncovered channels and in-flight work that cannot be recalled. Security actions use a deterministic path independent of an LLM and routine telemetry delivery.

After requesting containment, show each boundary's enforcement state: Requested, Enforced, Failed or Unconfirmed. Show reachability separately; for example, `Enforcement: Unconfirmed · Link: Disconnected`. A reachable gateway must never imply that unreachable nodes stopped. Keep uncertainty visible until authoritative receipts arrive.

Recovery requires the owner-defined recovery authority, fresh evidence and a new execution session. Quarantine is not cleared by a heartbeat, a better score or reconnecting. The actor under review cannot approve its own restoration. These semantics align with the proposed [swarm trust model](swarm-trust-model.md); they are not supplied by today's flight Stop control.

### Continue during degraded connectivity

| Condition | What the operator sees | Required behavior |
|---|---|---|
| Browser loses gateway | Gateway disconnected; last snapshot time; unsent draft preserved | Disable remote mutations; retain read-only snapshot; do not claim agent execution stopped. |
| Gateway loses an agent | Per-agent visibility lost; last reported state and collection gap | Retain the record as last known; do not turn it into “completed,” “idle” or silently remove it. |
| Internet unavailable, local mesh usable | Local services and models shown independently from cloud | Continue only work whose dependencies and policy allow local execution. |
| Policy authority/KAS unavailable | Specific protected operations unavailable; evidence/lease age where implemented | Do not convert cached availability into permission; runtime must enforce the relevant fail-closed policy. |
| Evidence stale or clock uncertainty excessive | Suspended or unable to determine eligibility, with reason | Local execution policy decides allowed work; UI does not extend leases. |
| Gateway restarts or event history has a gap | Reconnecting → reconciling → current/partial; missing interval visible | Recover durable state and command outcomes before enabling actions that depend on it. |

In the target trust design, already-authorized local processing may continue when policy permits; new remote key release, protected sharing and delegation need their required authority. Local custody and a provisioned swarm-local authority are distinct profiles. Do not describe all offline nodes as having identical rights.

The trust design proposes a maximum five-second positive lease and a connected containment propagation target of one second at p99. These are proposed enforcement acceptance targets, not measured product guarantees. UI telemetry freshness is a separate threshold and must not substitute for either.

Default to reduced traffic in degraded links: prioritize action receipts, security changes and mission state; coalesce diagnostic metrics; pause animations; provide explicit history ranges. Avoid high-frequency full snapshots for every hidden panel. Never buffer a privileged command in the browser for silent execution after reconnection.

### Reconstruct an outcome and hand over a shift

Evidence presents a correlated timeline with source, source time, receipt time, sequence/revision, subject, action, policy, outcome and artifact reference. Distinguish source order from cross-host wall-clock order. Deduplicate by event identity; show missing intervals rather than interpolating a clean history.

An authorized handover export records scope, time interval, unresolved decisions, current versus last-known states, evidence references and redactions. It is a backend-authorized artifact with provenance, not a dump of browser memory. Digest verification and collection completeness must be separate indicators: a valid signature does not prove a complete record.

## Trust and security presentation

Keep these dimensions separate in Swarm and Data & Trust:

| Dimension | Example question |
|---|---|
| Reachability | Can the gateway contact this agent now? |
| Identity | Which stable workload/session is this, and who verified it? |
| Membership | Who enrolled it, into which scope, and until when? |
| Authority | Which grant permits this action on this resource/destination? |
| Assurance | Is evidence self-reported, software-verified or independently hardware-appraised? |
| Security state | Unassessed, Eligible, Suspended or Quarantined under an implemented authority? |
| Coverage | Which execution and egress paths are actually mediated? |
| Evidence freshness | When was the relevant observation made, and is it still valid? |

The categorical trust states are target semantics. Until the runtime emits authoritative states, display **Not reported** instead of deriving them from uptime, KAS, signatures alone or reputation. Published trust scores may inform investigation or routing but cannot create authority.

Provide a capability coverage view with Supported and enforced, Configured but unverified, Not configured, Unsupported, and Unknown. Each entry links to its enforcing component and last relevant receipt. Unsupported hardware evidence must not be rendered as a failed attack; missing required evidence must not become permission.

## Visual and interaction direction

Use a quiet, precise operations workspace: charcoal/navy surfaces for low-light use, an equally legible daylight theme, restrained cyan for selection, amber for stale/blocked/unconfirmed, red for failed/denied/quarantined, and green only for a specifically verified success. Text and symbols always accompany color. No camouflage, radar animation, glowing borders or implied tactical authority.

Use system fonts and locally bundled assets. Body text should be at least 14 px in the default density; reserve monospace for identifiers, timestamps and measured values. A compact density changes spacing before font size. Use visible navigation labels and modest icons; keep one clear primary action per context.

Desktop uses a stable navigation column, primary workspace, and optional inspection rail. On tablets the rail becomes a dedicated detail view; narrow screens stack status and actions. Data tables may scroll within a labeled region when necessary, while the page itself reflows. Essential status must remain visible at 200% zoom.

Target WCAG 2.2 AA through explicit focus order, visible focus, text contrast, native buttons/forms, correct dialog focus trapping/restoration, Escape dismissal for noncommitted dialogs, and meaningful accessible names. Announce consequential status changes, not every telemetry sample. Respect reduced motion and never steal focus during polling.

Search should resolve mission, agent, flight, incident, policy and artifact IDs. Add it only when backed by the available data scope, with coverage disclosed. Keyboard shortcuts must be discoverable, avoid firing in text inputs and never directly execute destructive or privilege-changing commands.

Empty states distinguish first-run setup, no matching items, no reporting agents, unsupported capability, service error and data not yet received. Every error states the failed operation and next useful action. Avoid a perpetual “Loading…” state after a known disconnect.

The lexicon and information-help behavior are shared components. Keep visible copy to labels, facts, reasons and actions. Circled i controls reveal explanations on hover/focus and support pinned help on tap. Never require opening help to discover an unsafe or unavailable action. At narrow widths preserve the same field order with explicit labels; compact density reduces spacing, not essential content or legibility.

## Architecture and contracts

Keep the embedded web delivery model and Rust gateway. A build-time TypeScript client can improve event typing without introducing a Node runtime, CDN dependency or large framework on edge devices. Migration to a UI framework should be justified by maintenance needs and measured bundle cost, not visual redesign alone. Preserve same-origin protections, CSP, rustls and the no-OpenSSL requirement.

Separate browser responsibilities into transport/reconciliation, validated domain state, derived selectors, reusable accessible controls and workspace rendering. Keep authentication, policy evaluation, lease decisions, persistence and command execution in trusted backend components.

| Contract | Required fields/behavior | Existing fit |
|---|---|---|
| Capability descriptor | Supported operations, scope, enforcing component, schema version, evidence level and limitations | New; prevents UI controls from implying missing enforcement. |
| Snapshot envelope | Source instance, scope, revision, observed/received times, completeness and event cursor | Extend current mesh/status/ARP/context snapshots. |
| Command request | Operation/idempotency ID, target scope, expected revision, intent; authenticated actor derived server-side | New shared contract; adapt task and proposal commands. |
| Command receipt | Accepted/rejected, operation and resource IDs, reason code, applied revision, execution state | Existing result events are a starting point; transport send is not acceptance. |
| Mission record | Objective, authorized scope, constraints, child task/flight refs, state and result criteria | New durable parent; existing tasks remain valid children. |
| Security evidence | Issuer, subject/session binding, policy generation, evidence reference, validity and verification level | New aggregation over actual enforcement sources; never browser-generated. |
| Containment receipt | Incident, authority generation, target boundary, enforced time, unresolved/in-flight scope | New; registry deletion cannot satisfy it. |
| Audit event | Stable ID, source sequence, correlation IDs, event/receipt times, visibility rules and provenance | Extend current events; persist with bounded retention and explicit gaps. |

Use canonical AG-UI events for run/message/tool streaming. Keep Arkavo-specific operational snapshots and commands versioned; adapt existing WebSocket events during migration. AG-UI transport compatibility does not supply authorization or durable command semantics.

Unknown schema versions and malformed events must not crash the whole console or produce permissive state. Represent an unavailable field explicitly, preserve the last valid snapshot with a stale marker, and surface protocol errors without displaying secrets. Avoid logging raw chat/event payloads by default.

Do not store credentials, protected chat transcripts, evidence bodies or privileged command queues in browser localStorage. An unsent draft can survive transient disconnect in memory; reload persistence requires an explicit, policy-aware storage design. Render agent-controlled text as untrusted content, including names, rationales, tool output and evidence links.

## Delivery sequence

### Foundation: make current behavior honest

Implement the shared lexicon, state-to-label mappings and accessible information help. Make task submission connection-aware with visible outcomes; preserve drafts; fix dialog/navigation accessibility; introduce per-source freshness and unknown states; correct flight deregistration labels. Add regression tests before treating these as resolved. Establish the capability descriptor and separate available behavior from proposed enforcement.

Exit condition: disconnected/stale scenarios cannot look current or silently discard an action, and existing task/chat/proposal workflows remain functional.

### Operations workspace: expose existing value

Ship the new shell, Overview, Missions/task detail, Swarm and Decisions using real gateway data. Move existing health, cost, routing, learning and context panels into Runtime. Bring relevant policy/data references into mission detail. Add stable deep links and an accessible inspection pattern.

Exit condition: an operator can find a blocked task, explain its route, inspect relevant policy and review an existing ARP proposal without manually matching records across unrelated panels.

### Durable operation: survive interrupted sessions

Implement authenticated operator sessions and scoped grants; durable task/mission/command history; idempotency, action receipts, event cursors, gap detection and restart reconciliation. Resolve browser-to-operator authentication against the planned operator socket architecture. Do not expose broader remote administration while that boundary is unresolved.

Exit condition: a lost command reply, gateway restart and concurrent reviewers do not cause duplicate execution, lost intent or approval of stale state.

### Defensive control: expose verified enforcement

Integrate workload identity/enrollment, Guardian evidence, protected status authority, per-boundary containment, credential/egress controls and recovery workflows as those backend workstreams land. Add explicit capability/coverage reporting; enable only controls with complete authorization and receipt semantics.

Exit condition: fault-injection tests prove the stated boundary effects, including unreachable nodes and in-flight work. UI badges alone do not qualify this milestone.

### Field readiness: qualify constrained operation

Validate local dependencies and sealed assets before deployment; test link loss, high delay, low bandwidth, clock uncertainty, resource exhaustion and evidence recovery. Establish measured latency, binary size and frontend budgets on declared hardware. Prepare operator handover and evidence export.

Exit condition: a documented operating profile specifies what continues, what suspends and what cannot be guaranteed for each deployment mode.

Keep UI foundations in one cohesive change; stack subsequent topics from creation if split, with repository version rules applied to each implementation PR. This blueprint itself does not complete a software feature and does not change the package version.

## Acceptance and evaluation

These are proposed release gates and usability targets, not results from this review.

| Scenario | Required outcome |
|---|---|
| Disconnect before task send | Draft remains; no success message; no queued automatic dispatch. |
| Connection lost after dispatch | Outcome unknown until reconciled by operation ID; retry cannot duplicate execution. |
| Reconnect or gateway restart | Fresh snapshot/cursor reconciliation precedes dependent actions; history gaps are visible. |
| Agent disappears mid-task | Agent and task retain last-known state and age; neither becomes silently completed. |
| Missing KAS/policy/assurance data | Unknown or unavailable, never zero/healthy/eligible by default. |
| Stale proposal or competing reviewer | Server checks current revision and authority; stale approval fails with visible explanation. |
| Approval denied or failed | Decision remains visible with reason and authoritative result; no console-only failure. |
| Flight tracking removed | UI describes registry removal and makes no remote-stop or quarantine claim. |
| Containment requested on partitioned swarm | Per-boundary acknowledgments and unreachable nodes remain visible; no universal success. |
| Invalid evidence or compromised worker self-report | Cannot establish eligibility, authorize release or clear quarantine. |
| Internet lost, local dependencies present | Only policy-permitted local work continues; cloud and authority-dependent operations remain distinct. |
| Old/duplicate/out-of-order event | Idempotent reduction; newer state is not rolled back; event gaps are disclosed. |
| Malicious agent name/output/link | No script execution, attribute injection, unsafe navigation or secret leakage. |
| Keyboard-only / screen reader / 200% zoom | Main workflows usable; focus preserved across updates and dialogs; status not color-only. |
| High event volume / slow link | Commands and security outcomes remain usable; metrics coalesce; memory/history stay bounded. |
| Same concept across workspaces | One canonical noun, state and action label; no mission/task or accepted/completed substitutions. |
| Icons without color or with assets missing | State labels remain unambiguous; shape reinforces meaning in monochrome; navigation and actions work without icons. |
| Accepted / Completed / Enforced / Unknown / Information | Distinct symbols and labels; no inferred security guarantee; circled i opens help and question mark denotes missing knowledge. |
| Information help with mouse, keyboard or touch | Circled i opens on hover/focus, pins on tap and dismisses with Escape; explanation stays reachable and does not obscure its trigger. |
| Overview with help closed | Scope, blocker, age, missing evidence and unconfirmed outcomes are visible; a correct decision does not require reading hidden definitions. |
| Dense rows during live updates | Names, states and values remain aligned; focused rows do not jump as new items arrive. |

Test existing runtime paths with the crate's integration tests and new transport/state regression cases; test the production browser bundle in Chrome with real gateway contracts. Use fault injection only in the test harness, never fake production status. Apply the repository coverage requirement, formatting and warning-free linting to implementation changes, and the full pre-push security checklist before any push.

Suggested operator study: identify the highest-priority blocker within 5 seconds; distinguish Connected from Allowed and Requested from Enforced within 10 seconds; explain whether an agent may perform a specified protected action within 30 seconds; trace a result to its source and policy within 60 seconds. Use identical scenarios in the current UI and redesign; include first-time and experienced operators. Record first-choice accuracy, completion time, help openings and mistaken confidence. Target at least 90% correct first choices on ordinary scan tasks, and no permission or enforcement inferred from connectivity. These are usability targets to validate, not measured results. A confident incorrect “safe” answer is a design failure.

## Backend dependencies that must remain visible

| Dependency | Evidence / limitation |
|---|---|
| Agent ingress and membership | [INGRESS-002/003 and membership scenarios](../specs/arkavo-edge/agent-ingress.spec.yaml) retain WIP requirements; discovery is not enrollment. |
| Network and process confinement | [NET-003, NET-016, NET-019 and NET-020](../specs/arkavo-edge/network-security.spec.yaml) document incomplete authentication/TLS, confinement, egress and reply mediation paths. |
| Key lifecycle and offline security | [TDF security scenarios](../specs/arkavo-edge/tdf-security.spec.yaml) mark key persistence/rotation, freshness and other protections incomplete. |
| Attestation | [Hardware attestation scenarios](../specs/arkavo-edge/hardware-attestation.spec.yaml) contain WIP appraisal and freshness work; software claims cannot be relabeled as hardware assurance. |
| Classification and sealed knowledge | [Sentinel](../specs/arkavo-edge/sentinel.spec.yaml), [knowledge packs](../specs/arkavo-edge/knowledge-pack.spec.yaml) and [sequence integrity](../specs/arkavo-edge/sequence-integrity.spec.yaml) mix existing mechanisms with WIP integration. |
| Containment and recovery | [Swarm trust model](swarm-trust-model.md) defines proposed state, lease and enforcement contracts; the browser gateway does not currently implement that control plane. |

The first redesign should make the available product substantially easier to operate while making these boundaries more apparent. The later defensive control surface earns its claims through enforcement receipts, attributable decisions and tested behavior under failure.

## Runtime contract for the first implementation

The embedded web UI now uses the seven-workspace shell, a shared presentation lexicon, locally bundled Lucide 1.17.0 icons, and information help on hover, focus or click. Escape dismisses help; click pins it. The lexicon is also available as a keyboard-accessible dialog. All assets remain embedded in the Rust gateway; no browser package installation or runtime CDN is required. The icon license is served at `/static/icons/LICENSE`.

Overview derives its counts and attention queue from task, swarm and runtime-policy snapshots. Each displayed age is the browser's receipt age for the named gateway snapshot, not an assertion about the originating agent's clock or enforcement freshness. These snapshots poll every five seconds in the operational workspaces and become stale after fifteen seconds without a response, or immediately when the gateway disconnects. Missing data remains Unknown or Not reported until a response arrives.

`requestFleetSnapshot` returns the existing `meshStatus` payload using connection state rather than treating discovery as a successful link. It avoids remote key-service probes during routine swarm polling. The existing `requestMeshStatus` path remains available for service refresh when entering Data & Trust. Last-known agent records and chat drafts remain visible after an agent disappears. Closed WebSocket clients are reported as disconnected even when discovery connected them without a background reconnect monitor.

LAN discovery remains the default. `arkavo ui --agent IP:PORT` adds an explicit peer for environments where multicast is unavailable. The gateway reads its bounded agent card, rejects redirects and keeps the operator-selected endpoint; card metadata does not establish trust or authorization.

Missions currently contains standalone Tasks. Fresh task snapshots include the full stored output separately from the summary, so a reconnect or new browser can inspect completed work. The browser also retains lifecycle output, metrics and session evidence when an older gateway omits them. Numeric task progress is omitted because the existing gateway can estimate it from polling; the task's reported execution state remains visible. Task details identify energy as estimated.

Task creation sends an optional `requestId`, echoed only in its `taskSubmitted` receipt. This is correlation, not durable idempotency. A draft remains in its browser tab until the matching receipt arrives. Closing the composer keeps it; a lost connection or fifteen-second receipt timeout produces Outcome unknown and prevents automatic replay. Reloading the page or restarting the gateway does not provide durable reconciliation. The browser warns before discarding an outstanding draft on page exit.

Decisions exposes pending runtime-policy proposals with the exact reported scope, change, rationale and trace references. Its controls require a current policy snapshot and distinguish sending, gateway acceptance and rejection. The review is explicitly a local gateway review without a verified operator grant. End tracking removes a flight's gateway registry entry; it does not cancel or contain remote execution.

Evidence provides filtering, pause/resume and expandable raw records for the latest 200 events retained by the browser. It is marked session evidence and unverified. Health, resources, routing, adaptation, context and diagnostics remain reachable in Runtime; protected-data and runtime-policy detail remain in Data & Trust. Durable mission/run identity, persistent command receipts, authenticated reviewer grants and remote containment remain backend dependencies described above.

### Investigation workflow

Workspace sections sit beneath the page title. Tasks and agents have compact inspectors with links to assigned work, reported agent policy, decisions and session evidence. The task inspector keeps its objective, output and route visible, with metrics and supporting records behind disclosures. Definitions and estimation assumptions use the shared circled-i help. Missing output and metrics remain Not reported; a task summary is not substituted for an execution result.

Fragment routes preserve the selected task, agent, proposal or evidence scope. Browser Back restores the preceding route, workspace scroll position and originating record link when it still exists. Navigation positions and evidence text filters are bounded, in-memory session state; reload restores the route identifier, not cached evidence or durable history. A missing record remains explicitly unavailable at its requested identifier until a snapshot supplies it. Reconnecting refreshes the selected inspector without clearing its route.

Examples on the same gateway:

```text
#/tasks?task=<task-id>
#/agents?agent=<agent-id>
#/arp?agent=<agent-id>
#/decisions?agent=<agent-id>&proposal=<proposal-id>
#/telemetry?task=<task-id>
#/telemetry?agent=<agent-id>
#/telemetry?trace=<trace-id>
```

Production links encode identifiers and accept only the fields supported by each workspace. Task and trace evidence matches explicit identifier fields in the event or its structured details. Agent/time proximity and free-text mentions do not establish task correlation. An agent-policy link shows the currently reported agent document; the task's governing policy revision remains Not reported because the gateway does not provide that binding. An unavailable agent policy never silently selects another agent.

Evidence holds the displayed records while an event is expanded or keyboard focus is inside the records. New observations remain buffered within the existing retention limit; Show updates applies them explicitly. The UI reports the count of events evicted during this browser session. This count is a retention gap, not a gateway event cursor or proof that delivery was complete.

The investigation regression cases cover exact correlation, opaque identifiers, unavailable records, list/detail navigation, Back restoration, reconnect, disposition after review, retained chat drafts, missing metrics and stable evidence inspection. Chrome checks use the real gateway for routes, unavailable-record behavior, information help and live evidence at desktop and 390 px widths. Populated task, agent and proposal cases use test-only fixtures; no agents or task results are seeded into production.

### Implementation checks

Run the browser logic regression tests with Node's built-in runner:

```sh
node --test crates/arkavo-agui/tests/ui_operations.test.cjs
```

The tests cover receipt correlation, draft retention, disconnects, malformed frames, escaping, snapshot replacement, state vocabulary, help interactions, evidence filtering and review outcomes. They use test-only DOM and transport fixtures; production assets contain no seeded agents, tasks or policy decisions.

Run the focused Rust checks without a C++ inference dependency:

```sh
cargo fmt -- --check
cargo test -p arkavo-agui --no-default-features --features web-ui --lib --test ui_submission --test connection_regression --test metrics_integration
cargo clippy -p arkavo-agui --no-default-features --features web-ui --lib --test ui_submission -- -D warnings
```

Chrome verification should include desktop and narrow viewports, modal focus and help, retained drafts, evidence filtering/pause, workspace navigation, gateway shutdown/reconnect and visible stale observations. Test populated task and review states through the regression fixtures when no live agents are available. A complete release also requires the repository's full build, security and pre-push checks; these focused checks do not replace them.
