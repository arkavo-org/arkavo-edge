// `#[tokio::test]` expands to `Runtime::block_on`, which the workspace lint
// set disallows in library code.
#![allow(clippy::disallowed_methods)]

use std::collections::HashMap;
use std::sync::Mutex;

use super::*;
use arkavo_swarmkit_delegation::{
    CapabilityReport, Policy, RefusalReason, RevokeError, Specialist, StateStore,
};
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
    /// Commits a role, then loses the acknowledgement, as a timeout does.
    drops_acks: bool,
    /// Loses the connection before it reads the envelope.
    unreachable: bool,
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
            drops_acks: false,
            unreachable: false,
            _dir: dir,
        }
    }

    fn dropping_acks(self) -> Self {
        Self {
            drops_acks: true,
            ..self
        }
    }

    fn unreachable(self) -> Self {
        Self {
            unreachable: true,
            ..self
        }
    }

    fn key(&self) -> String {
        self.specialist.lock().unwrap().key().to_string()
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
        if host.unreachable {
            return Err(Failure::Unreachable("connection reset".into()));
        }
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
        let ready = specialist.commit(admitted, Utc::now()).unwrap();
        drop(specialist);
        if host.drops_acks {
            return Err(Failure::Unreachable("timed out".into()));
        }
        Ok(ready)
    }

    fn give_back(&self, address: &str, revocation: serde_json::Value) -> Result<(), Failure> {
        let mut specialist = self.0[address].specialist.lock().unwrap();
        specialist
            .revoke(revocation, Utc::now())
            .map(drop)
            .map_err(|e| match e {
                RevokeError::Refused(refusal) => Failure::Refused(refusal),
                RevokeError::Store(e) => Failure::Unreachable(e.to_string()),
            })
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

    fn status(
        &self,
        address: &str,
    ) -> impl Future<Output = Result<SpecialistStatus, Failure>> + Send {
        std::future::ready(Ok(self.0[address].status()))
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
    let applied = assign(
        &manifest(),
        &me,
        &net.candidates(&did),
        TimeDelta::days(7),
        Utc::now(),
        &net,
    )
    .await
    .unwrap();
    let (flight, assignments) = (applied.flight, applied.assignments);
    assert_eq!(applied.cleaned_up, []);

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
    assert_eq!(unassigned.rolled_back, [restored("a", "artist")]);
    assert_eq!(net2.0["a"].role(), None, "the artist role was taken back");
    assert!(
        net2.0["a"].status().binding.is_none(),
        "an agent unbound before the apply is unbound after it"
    );
}

fn restored(agent: &str, role: &str) -> RolledBack {
    RolledBack {
        agent: agent.into(),
        role: role.into(),
        undo: Undo::Restored,
    }
}

/// Bind `host` to `me` with `role_id` of `manifest` under `flight`, issued
/// ten minutes ago, as an earlier apply would have.
fn hold_role(
    net: &Net,
    name: &str,
    me: &AgentKeypair,
    manifest: &Manifest,
    role_id: &str,
    flight: Uuid,
) {
    let role = manifest.roles.iter().find(|r| r.id == role_id).unwrap();
    let signed = Delegation::for_role(
        manifest,
        role,
        me.public_key().to_did_key(),
        net.0[name].key(),
        flight,
        Utc::now() - TimeDelta::minutes(10),
        TimeDelta::days(1),
    )
    .unwrap()
    .sign(me)
    .unwrap();
    net.take_role(name, signed.wire().clone()).unwrap();
}

/// Review finding: a failed re-apply took down a role an agent held from
/// an earlier apply. Undoing gives it back when it came from the same kit,
/// and says it is gone when it came from another.
#[spec("SKD-012")]
#[tokio::test]
async fn a_failed_reapply_gives_back_the_earlier_role() {
    let me = AgentKeypair::generate();
    let did = me.public_key().to_did_key();
    let net = net(vec![("a", Host::new(true, Some(true), 0.0))]);
    let earlier = Uuid::new_v4();
    hold_role(&net, "a", &me, &manifest(), "writer", earlier);

    let unassigned = assign(
        &manifest(),
        &me,
        &net.candidates(&did),
        TimeDelta::days(1),
        Utc::now(),
        &net,
    )
    .await
    .unwrap_err();
    assert_eq!(unassigned.role, "writer", "the artist took the only agent");
    assert_eq!(unassigned.rolled_back, [restored("a", "artist")]);
    assert_eq!(net.0["a"].role().as_deref(), Some("writer"));
    let status = net.0["a"].status();
    assert_eq!(status.delegation.unwrap().flight_id, earlier);
    assert_eq!(status.binding.unwrap().orchestrator, did);

    let mut other_kit = manifest();
    other_kit.kit.id = "blake3:another-kit".into();
    let net = self::net(vec![("a", Host::new(true, Some(true), 0.0))]);
    hold_role(&net, "a", &me, &other_kit, "writer", Uuid::new_v4());
    let unassigned = assign(
        &manifest(),
        &me,
        &net.candidates(&did),
        TimeDelta::days(1),
        Utc::now(),
        &net,
    )
    .await
    .unwrap_err();
    assert_eq!(
        unassigned.rolled_back[0].undo,
        Undo::EarlierRoleLost {
            role: "writer".into(),
            kit_id: "blake3:another-kit".into()
        }
    );
    assert_eq!(net.0["a"].role(), None);
}

/// Review finding: an agent that never got this flight's role still holds
/// its earlier one from another kit, so undoing does not report it lost.
#[spec("SKD-012")]
#[tokio::test]
async fn an_agent_that_never_took_the_role_keeps_its_other_kits_role() {
    let me = AgentKeypair::generate();
    let did = me.public_key().to_did_key();
    let mut other_kit = manifest();
    other_kit.kit.id = "blake3:another-kit".into();
    let net = net(vec![("a", Host::new(true, Some(true), 0.0))]);
    hold_role(&net, "a", &me, &other_kit, "writer", Uuid::new_v4());
    let candidates = net.candidates(&did);
    let net = Net(net
        .0
        .into_iter()
        .map(|(n, h)| (n, h.unreachable()))
        .collect());

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
    assert_eq!(unassigned.rolled_back, [restored("a", "artist")]);
    assert_eq!(net.0["a"].role().as_deref(), Some("writer"));
}

/// Review finding: an agent that committed a role whose acknowledgement
/// was lost kept it. Undoing reaches it too, whether the apply fails or
/// the role went to another agent.
#[spec("SKD-012")]
#[tokio::test]
async fn an_unconfirmed_offer_is_taken_back() {
    let me = AgentKeypair::generate();
    let did = me.public_key().to_did_key();
    let net = net(vec![
        ("x", Host::new(true, Some(true), 0.0).dropping_acks()),
        ("y", Host::new(true, None, 0.0)),
    ]);
    let unassigned = assign(
        &manifest(),
        &me,
        &net.candidates(&did),
        TimeDelta::days(1),
        Utc::now(),
        &net,
    )
    .await
    .unwrap_err();
    assert_eq!(unassigned.role, "artist");
    assert_eq!(unassigned.rolled_back, [restored("x", "artist")]);
    assert_eq!(net.0["x"].role(), None);
    assert!(net.0["x"].status().binding.is_none());

    let net = self::net(vec![
        ("x", Host::new(true, Some(true), 0.0).dropping_acks()),
        ("y", Host::new(true, None, 0.0)),
        ("z", Host::new(true, Some(false), 0.0)),
    ]);
    let applied = assign(
        &manifest(),
        &me,
        &net.candidates(&did),
        TimeDelta::days(1),
        Utc::now(),
        &net,
    )
    .await
    .unwrap();
    let by_role: HashMap<_, _> = applied
        .assignments
        .iter()
        .map(|a| (a.role.as_str(), a.agent.as_str()))
        .collect();
    assert_eq!((by_role["artist"], by_role["writer"]), ("z", "y"));
    assert_eq!(applied.cleaned_up, [restored("x", "artist")]);
    assert_eq!(
        net.0["x"].role(),
        None,
        "x took the writer role silently, then gave it back"
    );
    assert!(net.0["x"].status().binding.is_none());
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
