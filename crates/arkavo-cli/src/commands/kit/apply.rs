//! `arkavo kit apply`: hand a kit's roles to agents already running on the
//! network (SwarmKit draft-02 §7.1.2, SKD-012).
//!
//! The orchestrator is this host's agent identity key. Agents started
//! without a kit bind to the first orchestrator that assigns them a role,
//! so the first apply on a network decides which host the agents answer to.

use std::io::Write;
use std::path::Path;

use arkavo_protocol::http::HttpTransport;
use arkavo_protocol::transport::{
    A2aEndpoint, A2aRequest, A2aResponse, A2aTransport, TlsConfig, TransportConfig,
};
use arkavo_swarmkit_delegation::{
    MAX_LIFETIME, REFUSAL_CODE, Ready, Refusal, SpecialistStatus, methods,
};
use chrono::{TimeDelta, Utc};
use serde_json::{Value, json};

use super::{reject_unknown_options, usage_error, validate_kit_at};

mod flight;

use flight::{
    Applied, Candidate, Failure, RolledBack, Specialists, Unassigned, Undo, assign, eligibility,
};

const HELP: &str = "arkavo kit apply --help";

/// How long each role is handed out for unless `--days` says otherwise.
const DEFAULT_DAYS: i64 = 7;

pub(super) fn cmd_apply(
    _base_dir: &Path,
    args: &[String],
    out: &mut dyn Write,
) -> Result<(), Box<dyn std::error::Error>> {
    let (path, days) = parse(args)?;
    let lifetime = TimeDelta::days(days);
    let now = Utc::now();
    let report = validate_kit_at(path, now)?;
    if !report.id_matches {
        return Err(format!(
            "{}: kit.id is not set; set it to {} first",
            path.display(),
            report.computed_id
        )
        .into());
    }
    if !report.pipelines.is_empty() {
        return Err(format!(
            "{}: a pipeline kit's roles find each other by role id; start each with \
             'arkavo agent -c <path> -n <role>'",
            path.display()
        )
        .into());
    }
    let manifest = arkavo_swarmkit::read_kit_file(path)?.manifest;
    let keypair = arkavo_registration::load_or_create_agent_keypair()?;
    let me = keypair.public_key().to_did_key();
    writeln!(out, "Orchestrator {me}")?;

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    #[allow(clippy::disallowed_methods)]
    let outcome = runtime.block_on(async {
        let (candidates, lines) = discover(&me).await?;
        for line in lines {
            writeln!(out, "{line}")?;
        }
        Ok::<_, Box<dyn std::error::Error>>(
            assign(&manifest, &keypair, &candidates, lifetime, now, &Http).await,
        )
    })?;
    match outcome {
        Ok(applied) => {
            print_applied(out, &manifest.kit.name, &applied)?;
            Ok(())
        }
        Err(unassigned) => Err(describe(&unassigned).into()),
    }
}

fn parse(args: &[String]) -> Result<(&Path, i64), Box<dyn std::error::Error>> {
    let mut path = None;
    let mut days = DEFAULT_DAYS;
    let mut rest = args.iter();
    while let Some(arg) = rest.next() {
        if arg == "--days" {
            days = rest
                .next()
                .and_then(|d| d.parse::<i64>().ok())
                .filter(|d| (1..=MAX_LIFETIME.num_days()).contains(d))
                .ok_or_else(|| usage_error("--days takes a whole number from 1 to 30", HELP))?;
        } else if path.is_none() && !arg.starts_with('-') {
            path = Some(Path::new(arg.as_str()));
        } else {
            reject_unknown_options(std::slice::from_ref(arg), HELP)?;
            return Err(usage_error(&format!("unexpected argument '{arg}'"), HELP));
        }
    }
    Ok((
        path.ok_or_else(|| usage_error("kit path required", HELP))?,
        days,
    ))
}

/// Agents on the network that answer `swarmkit/status`, one per key, and a
/// line about each agent found, naming why one cannot take a role from `me`.
async fn discover(me: &str) -> Result<(Vec<Candidate>, Vec<String>), String> {
    let agents = tokio::task::spawn_blocking(|| {
        use crate::commands::mesh::{Listen, discover_mesh_agents_with};
        discover_mesh_agents_with(Listen::ALL).map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())??;
    let mut candidates: Vec<Candidate> = Vec::new();
    let mut lines = vec!["Agents on the network:".to_string()];
    for agent in agents {
        let Some(address) = agent.address else {
            continue;
        };
        let name = agent.agent_id;
        let status = match call(&address, &name, methods::STATUS, json!([])).await {
            Ok(value) => {
                serde_json::from_value::<SpecialistStatus>(value).map_err(|e| e.to_string())
            }
            Err(failure) => Err(failure.to_string()),
        };
        let line = match status {
            Err(reason) => format!("skipped: {reason}"),
            Ok(status)
                if candidates
                    .iter()
                    .any(|c| c.status.specialist == status.specialist) =>
            {
                "skipped: another process with the same key was found first".into()
            }
            Ok(status) => match eligibility(&status, me) {
                Err(reason) => format!("skipped: {reason}"),
                Ok(()) => {
                    let line = describe_candidate(&status);
                    candidates.push(Candidate {
                        name: name.clone(),
                        address: address.clone(),
                        status,
                    });
                    line
                }
            },
        };
        lines.push(format!("  {name}  {address}  {line}"));
    }
    Ok((candidates, lines))
}

fn describe_candidate(status: &SpecialistStatus) -> String {
    let capabilities: Vec<String> = status
        .capabilities
        .iter()
        .map(|c| format!("{}{}", c.name, if c.ready { "" } else { " (not ready)" }))
        .collect();
    let role = status
        .delegation
        .as_ref()
        .map_or_else(|| "no role".to_string(), |d| format!("role {}", d.role_id));
    let mut line = format!(
        "{}, {role}",
        if status.binding.is_some() {
            "bound to this orchestrator"
        } else {
            "unbound"
        }
    );
    if !capabilities.is_empty() {
        line = format!("{line}, {}", capabilities.join(", "));
    }
    line
}

fn print_applied(out: &mut dyn Write, kit: &str, applied: &Applied) -> std::io::Result<()> {
    writeln!(out, "Applied {kit} as flight {}:", applied.flight)?;
    for a in &applied.assignments {
        writeln!(
            out,
            "  {} -> {} ({}), until {}",
            a.role, a.agent, a.specialist, a.expires
        )?;
    }
    for line in applied.cleaned_up.iter().map(describe_undo) {
        writeln!(out, "{line}")?;
    }
    writeln!(
        out,
        "Agents keep their roles across restarts until then. Run apply again to reassign them."
    )
}

/// What putting one agent back did, as a line of output.
fn describe_undo(rolled_back: &RolledBack) -> String {
    let RolledBack { agent, role, undo } = rolled_back;
    match undo {
        Undo::Restored => format!("  {agent}: put back as it was (offered {role})"),
        Undo::EarlierRoleLost {
            role: earlier,
            kit_id,
        } => format!(
            "  {agent}: took role {role} back, but its earlier role {earlier} came from kit \
             {kit_id}, which this apply cannot give back; apply that kit again"
        ),
        Undo::Failed(failure) => {
            format!("  {agent}: may still hold role {role}; could not take it back: {failure}")
        }
    }
}

fn describe(unassigned: &Unassigned) -> String {
    let mut lines = vec![format!("no agent took role {}", unassigned.role)];
    if unassigned.offers.is_empty() {
        lines[0].push_str(": no agent on the network can take it");
    }
    for (agent, failure) in &unassigned.offers {
        lines.push(format!("  {agent}: {failure}"));
    }
    lines.extend(unassigned.rolled_back.iter().map(describe_undo));
    lines.join("\n")
}

/// Agents reached over JSON-RPC on HTTP.
struct Http;

impl Specialists for Http {
    async fn delegate(&self, address: &str, envelope: Value) -> Result<Ready, Failure> {
        let value = call(
            address,
            address,
            methods::DELEGATE,
            json!({"envelope": envelope}),
        )
        .await?;
        serde_json::from_value(value)
            .map_err(|e| Failure::Unreachable(format!("unexpected answer: {e}")))
    }

    async fn status(&self, address: &str) -> Result<SpecialistStatus, Failure> {
        let value = call(address, address, methods::STATUS, json!([])).await?;
        serde_json::from_value(value)
            .map_err(|e| Failure::Unreachable(format!("unexpected answer: {e}")))
    }

    async fn revoke(&self, address: &str, revocation: Value) -> Result<(), Failure> {
        call(
            address,
            address,
            methods::REVOKE,
            json!({"revocation": revocation}),
        )
        .await
        .map(drop)
    }
}

async fn call(
    address: &str,
    agent_id: &str,
    method: &str,
    params: Value,
) -> Result<Value, Failure> {
    let unreachable = |e: &dyn std::fmt::Display| Failure::Unreachable(e.to_string());
    let transport = HttpTransport::new(TransportConfig {
        timeout_ms: 15_000,
        max_retries: 0,
        tls_config: TlsConfig {
            require_tls: false,
            ..Default::default()
        },
        ..Default::default()
    })
    .map_err(|e| unreachable(&e))?;
    let endpoint = A2aEndpoint {
        url: address.to_string(),
        agent_id: agent_id.to_string(),
        public_key: None,
    };
    transport
        .connect(&endpoint)
        .await
        .map_err(|e| unreachable(&e))?;
    let response = transport
        .send_request(A2aRequest::new(method, params))
        .await
        .map_err(|e| unreachable(&e));
    transport.close().await.ok();
    match response? {
        A2aResponse::Success { result, .. } => Ok(result),
        A2aResponse::Error { error, .. } if error.code == REFUSAL_CODE => Err(error
            .data
            .and_then(|data| serde_json::from_value::<Refusal>(data).ok())
            .map_or_else(|| Failure::Unreachable(error.message), Failure::Refused)),
        // jsonrpsee's own answer for a method the server does not have.
        A2aResponse::Error { error, .. }
            if error.code == -32601 && error.message == "Method not found" =>
        {
            Err(Failure::Unreachable(format!(
                "does not answer {method}: an arkavo release before kit roles (0.104), or not arkavo"
            )))
        }
        A2aResponse::Error { error, .. } => Err(Failure::Unreachable(format!(
            "{} ({})",
            error.message, error.code
        ))),
    }
}
