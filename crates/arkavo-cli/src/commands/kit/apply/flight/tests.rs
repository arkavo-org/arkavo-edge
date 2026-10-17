// `#[tokio::test]` expands to `Runtime::block_on`, which the workspace lint
// set disallows in library code.
#![allow(clippy::disallowed_methods)]

use std::collections::HashMap;
use std::sync::Mutex;

use super::*;
use arkavo_swarmkit_delegation::{CapabilityReport, Policy, RefusalReason, Specialist, StateStore};
use arkavo_test_macros::spec;

const KIT: &str = r#"
spec_version: "1.0.0"
kit:
  id: ""
  name: image-swarm
  version: 0.1.0
  authors: [{did: "did:web:local.arkavo.dev"}]
  created: "2026-10-16T00:00:00Z"
  expires: "2027-01-14T00:00:00Z"
  nonce: "ZgOj5i5lHm-ZxfrCwEr5rQ"
objective:
  goal: Make game art on request
roles:
  - id: writer
    role_type: operator
    agent_provisioning:
      model: {family: gemma, size: E2B}
    mcp_tools:
      - {server: arkavo, auth: none, tools: [generate_image]}
  - id: artist
    role_type: operator
    agent_provisioning:
      model: {family: gemma, size: E2B}
    mcp_tools:
      - {server: arkavo, auth: none, tools: [generate_image, image_generation]}
coordination: {topology: hub-spoke, protocol: a2a-jsonrpc-2.0, routing: {strategy: static}}
constraints:
  global_budget: {max_wallclock_seconds: 1800, max_total_tokens: 100000, max_cost_usd: 0.0}
  network: {egress_allowed: false}
completion: {rules: [objective addressed], on_failure: abort, max_retries: 0}
provenance: {signatures: []}
"#;

fn manifest() -> Manifest {
    let mut manifest: Manifest = serde_yaml::from_str(KIT).unwrap();
    manifest.kit.id = arkavo_swarmkit::kit_id_for(&manifest).unwrap();
    manifest
}

/// An agent on the test network: a real specialist, and whether its host
/// can generate images.
struct Host {
    specialist: Mutex<Specialist>,
    generates: Option<bool>,
    load: f64,
    _dir: tempfile::TempDir,
}

impl Host {
    fn new(first_use: bool, generates: Option<bool>, load: f64) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let key = AgentKeypair::generate().public_key().to_did_key();
        let specialist = Specialist::open(
            key,
            StateStore::new(dir.path()),
            Policy { first_use },
            Utc::now(),
        )
        .unwrap();
        Self {
            specialist: Mutex::new(specialist),
            generates,
            load,
            _dir: dir,
        }
    }

    fn status(&self) -> SpecialistStatus {
        let capabilities = self
            .generates
            .map(|ready| CapabilityReport {
                name: "image_generation".into(),
                ready,
            })
            .into_iter()
            .collect();
        self.specialist
            .lock()
            .unwrap()
            .status(capabilities, self.load)
    }

    fn role(&self) -> Option<String> {
        self.specialist
            .lock()
            .unwrap()
            .active()
            .map(|d| d.role_id.clone())
    }
}

struct Net(HashMap<String, Host>);

impl Net {
    fn candidates(&self, me: &str) -> Vec<Candidate> {
        let mut candidates: Vec<Candidate> = self
            .0
            .iter()
            .map(|(name, host)| Candidate {
                name: name.clone(),
                address: name.clone(),
                status: host.status(),
            })
            .filter(|c| eligibility(&c.status, me).is_ok())
            .collect();
        candidates.sort_by(|a, b| a.name.cmp(&b.name));
        candidates
    }
}

impl Net {
    fn take_role(&self, address: &str, envelope: serde_json::Value) -> Result<Ready, Failure> {
        let host = &self.0[address];
        let mut specialist = host.specialist.lock().unwrap();
        let admitted = specialist
            .admit(envelope, Utc::now())
            .map_err(Failure::Refused)?;
        let needs_generation = admitted
            .delegation()
            .host_requirements
            .iter()
            .any(|c| c == "image_generation");
        if needs_generation && host.generates.is_none() {
            return Err(Failure::Refused(Refusal::new(
                RefusalReason::HostUnsuitable,
                "this host cannot generate images",
            )));
        }
        Ok(specialist.commit(admitted, Utc::now()).unwrap())
    }

    fn give_back(&self, address: &str, revocation: serde_json::Value) -> Result<(), Failure> {
        let mut specialist = self.0[address].specialist.lock().unwrap();
        specialist
            .revoke(revocation, Utc::now())
            .map(drop)
            .map_err(|e| Failure::Unreachable(e.to_string()))
    }
}

impl Specialists for Net {
    fn delegate(
        &self,
        address: &str,
        envelope: serde_json::Value,
    ) -> impl Future<Output = Result<Ready, Failure>> + Send {
        std::future::ready(self.take_role(address, envelope))
    }

    fn revoke(
        &self,
        address: &str,
        revocation: serde_json::Value,
    ) -> impl Future<Output = Result<(), Failure>> + Send {
        std::future::ready(self.give_back(address, revocation))
    }
}

fn net(hosts: Vec<(&str, Host)>) -> Net {
    Net(hosts.into_iter().map(|(n, h)| (n.to_string(), h)).collect())
}

#[spec("SKD-012")]
#[tokio::test]
async fn the_artist_goes_to_the_ready_generator_and_every_agent_binds_to_me() {
    let me = AgentKeypair::generate();
    let did = me.public_key().to_did_key();
    let net = net(vec![
        ("macbook", Host::new(true, Some(false), 0.0)),
        ("m6", Host::new(true, Some(true), 0.5)),
    ]);
    let (flight, assignments) = assign(
        &manifest(),
        &me,
        &net.candidates(&did),
        TimeDelta::days(7),
        Utc::now(),
        &net,
    )
    .await
    .unwrap();

    let by_role: HashMap<_, _> = assignments
        .iter()
        .map(|a| (a.role.as_str(), a.agent.as_str()))
        .collect();
    assert_eq!(by_role["artist"], "m6", "ready weights beat the lower load");
    assert_eq!(by_role["writer"], "macbook");
    assert_eq!(net.0["m6"].role().as_deref(), Some("artist"));
    assert_eq!(net.0["macbook"].role().as_deref(), Some("writer"));
    for host in net.0.values() {
        let status = host.status();
        assert_eq!(status.binding.unwrap().orchestrator, did);
        assert_eq!(status.delegation.unwrap().flight_id, flight);
    }
}

#[spec("SKD-012")]
#[tokio::test]
async fn a_refusal_moves_the_role_and_a_role_nobody_takes_undoes_the_rest() {
    let me = AgentKeypair::generate();
    let did = me.public_key().to_did_key();
    // The only host that reports generation then refuses it, so the artist
    // finds nobody, after the writer was handed out.
    let net = net(vec![("pi", Host::new(true, None, 0.0))]);
    let mut candidates = net.candidates(&did);
    candidates[0].status.capabilities.push(CapabilityReport {
        name: "image_generation".into(),
        ready: true,
    });
    let unassigned = assign(
        &manifest(),
        &me,
        &candidates,
        TimeDelta::days(1),
        Utc::now(),
        &net,
    )
    .await
    .unwrap_err();
    assert_eq!(unassigned.role, "artist");
    assert_eq!(unassigned.offers.len(), 1);
    assert!(
        matches!(&unassigned.offers[0].1, Failure::Refused(r) if r.reason == RefusalReason::HostUnsuitable)
    );
    assert_eq!(
        unassigned.rolled_back.len(),
        0,
        "the artist goes first, so nothing was handed out"
    );

    let net2 = self::net(vec![("a", Host::new(true, Some(true), 0.0))]);
    let unassigned = assign(
        &manifest(),
        &me,
        &net2.candidates(&did),
        TimeDelta::days(1),
        Utc::now(),
        &net2,
    )
    .await
    .unwrap_err();
    assert_eq!(unassigned.role, "writer", "one agent, two roles");
    assert_eq!(unassigned.offers.len(), 0);
    assert_eq!(unassigned.rolled_back, [("artist".to_string(), None)]);
    assert_eq!(net2.0["a"].role(), None, "the artist role was taken back");
}

#[spec("SKD-012")]
#[test]
fn candidates_are_agents_that_bind_to_me_ranked_by_readiness_load_and_name() {
    let me = "did:key:z6Mkme";
    let host = |first_use| Host::new(first_use, None, 0.0);
    let closed = host(false).status();
    assert!(
        eligibility(&closed, me).is_err(),
        "a kit agent takes no role"
    );
    assert!(eligibility(&host(true).status(), me).is_ok());

    let candidate = |name: &str, generates: Option<bool>, load: f64| Candidate {
        name: name.into(),
        address: name.into(),
        status: Host::new(true, generates, load).status(),
    };
    let candidates = vec![
        candidate("b", Some(false), 0.0),
        candidate("c", Some(true), 0.9),
        candidate("a", Some(true), 0.9),
        candidate("d", None, 0.0),
    ];
    let names = |ranked: Vec<&Candidate>| ranked.iter().map(|c| c.name.clone()).collect::<Vec<_>>();
    let needs = vec!["image_generation".to_string()];
    assert_eq!(
        names(ranked(&needs, &candidates, &HashSet::new())),
        ["a", "c", "b"]
    );
    let taken = HashSet::from([candidates[2].status.specialist.clone()]);
    assert_eq!(names(ranked(&needs, &candidates, &taken)), ["c", "b"]);
    assert_eq!(
        names(ranked(&[], &candidates, &HashSet::new())),
        ["b", "d", "a", "c"]
    );

    let m = manifest();
    let order: Vec<&str> = role_order(&m.roles).iter().map(|r| r.id.as_str()).collect();
    assert_eq!(order, ["artist", "writer"]);
}
