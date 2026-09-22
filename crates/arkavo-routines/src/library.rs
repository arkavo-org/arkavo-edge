use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::{Result, Routine};

#[path = "store.rs"]
mod store;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Record {
    pub routine: Routine,
    pub successes: u32,
    pub failures: u32,
    pub retired: bool,
    in_flight: bool,
    consecutive_failures: u32,
    evidence_sessions: BTreeSet<String>,
}

impl Record {
    pub fn active(&self) -> bool {
        !self.retired && !self.in_flight && self.evidence_sessions.len() >= 2
    }

    pub fn confidence(&self) -> f64 {
        (f64::from(self.successes) + 1.0)
            / (f64::from(self.successes) + f64::from(self.failures) + 2.0)
    }
}

/// One library per agent and trust domain. Retired entries consume capacity
/// too, so bounded storage never forgets a rejection and relearns it silently.
#[derive(Default)]
pub struct Library {
    records: BTreeMap<String, Record>,
    path: Option<std::path::PathBuf>,
    unavailable: bool,
    // The open file owns the OS lease for this agent's durable library.
    lease: Option<std::fs::File>,
}

impl Library {
    pub fn open(path: &std::path::Path) -> Result<Self> {
        if path.is_dir() {
            return Err("Routine store must be a file".into());
        }
        let lease = store::lock(path)?;
        let mut library = match store::read(path)? {
            Some(bytes) => Self::restore(&bytes)?,
            None => Self::default(),
        };
        library.path = Some(path.into());
        library.lease = Some(lease);
        library.persist()?;
        Ok(library)
    }

    pub fn catalog(&self) -> &BTreeMap<String, Record> {
        &self.records
    }

    pub(crate) fn admit(&mut self, routine: Routine, session: &str) -> Result<String> {
        if self.unavailable {
            return Err("Routine persistence is unavailable".into());
        }
        routine.validate()?;
        let id = Self::id(&routine)?;
        if !self.records.contains_key(&id) && self.records.len() >= 128 {
            return Err("Routine library is full".into());
        }
        let record = self.records.entry(id.clone()).or_insert_with(|| Record {
            routine,
            successes: 0,
            failures: 0,
            retired: false,
            in_flight: false,
            consecutive_failures: 0,
            evidence_sessions: BTreeSet::new(),
        });
        if record.retired {
            return Err("This routine version is retired".into());
        }
        // Two separate task sessions must demonstrate the template. Repeating
        // a learn call over the same evidence cannot manufacture confidence.
        if record.evidence_sessions.len() < 2 && record.evidence_sessions.insert(session.into()) {
            record.successes = record.successes.saturating_add(1);
        }
        self.persist()?;
        Ok(id)
    }

    pub(crate) fn resolve(&self, id: &str) -> Result<Routine> {
        if self.unavailable {
            return Err("Routine persistence is unavailable".into());
        }
        self.records
            .get(id)
            .filter(|r| r.active() && !r.in_flight)
            .map(|r| r.routine.clone())
            .ok_or_else(|| "Routine is unknown, provisional or retired".into())
    }

    pub(crate) fn begin(&mut self, id: &str) -> Result<()> {
        self.resolve(id)?;
        self.records
            .get_mut(id)
            .ok_or("Routine unavailable")?
            .in_flight = true;
        // Persist before side effects. Restart after interrupted replay retires
        // the routine instead of silently forgetting an unknown outcome.
        self.persist()
    }

    pub(crate) fn outcome(&mut self, id: &str, success: bool) -> Result<()> {
        if let Some(record) = self.records.get_mut(id) {
            record.in_flight = false;
            if success {
                record.successes = record.successes.saturating_add(1);
                record.consecutive_failures = 0;
            } else {
                record.failures = record.failures.saturating_add(1);
                record.consecutive_failures = record.consecutive_failures.saturating_add(1);
                if record.consecutive_failures >= 2 || record.confidence() < 0.5 {
                    record.retired = true;
                }
            }
        }
        self.persist()
    }

    pub(crate) fn interrupt(&mut self, id: &str) -> Result<()> {
        if let Some(record) = self.records.get_mut(id) {
            record.retired = true;
        }
        self.outcome(id, false)
    }

    /// Export only templates, evidence identifiers and lifecycle counters.
    /// Hosts are responsible for private, atomic storage within the same trust domain.
    pub fn snapshot(&self) -> Result<Vec<u8>> {
        serde_json::to_vec(&serde_json::json!({"version":1,"records":self.records}))
            .map_err(|e| e.to_string())
    }

    pub fn restore(bytes: &[u8]) -> Result<Self> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Snapshot {
            version: u32,
            records: BTreeMap<String, Record>,
        }
        if bytes.len() > 3_000_000 {
            return Err("Routine snapshot exceeds limit".into());
        }
        let snapshot: Snapshot = serde_json::from_slice(bytes).map_err(|e| e.to_string())?;
        if snapshot.version != 1 || snapshot.records.len() > 128 {
            return Err("Unsupported or oversized routine snapshot".into());
        }
        let mut records = snapshot.records;
        for (id, record) in &mut records {
            record.routine.validate()?;
            if *id != Self::id(&record.routine)?
                || record.evidence_sessions.len() > 2
                || record
                    .evidence_sessions
                    .iter()
                    .any(|s| uuid::Uuid::parse_str(s).is_err())
                || (record.consecutive_failures >= 2 && !record.retired)
            {
                return Err("Invalid routine record".into());
            }
            if record.in_flight {
                record.retired = true;
                record.in_flight = false;
            }
        }
        Ok(Self {
            records,
            path: None,
            unavailable: false,
            lease: None,
        })
    }

    fn persist(&mut self) -> Result<()> {
        if let Some(path) = &self.path {
            if self.lease.is_none() {
                self.unavailable = true;
                return Err("Routine store lease is unavailable".into());
            }
            let result = store::write(path, &self.snapshot()?);
            if result.is_err() {
                self.unavailable = true;
            }
            result?;
        }
        Ok(())
    }

    fn id(routine: &Routine) -> Result<String> {
        // Alpha-renaming an input must not evade a retired-version record.
        fn normalize(value: &mut serde_json::Value, names: &mut BTreeMap<String, String>) {
            match value {
                serde_json::Value::Object(fields) => {
                    if let Some(serde_json::Value::String(name)) = fields.get_mut("$input") {
                        let next = format!("p{}", names.len());
                        let original = name.clone();
                        name.clone_from(names.entry(original).or_insert(next));
                    } else {
                        for value in fields.values_mut() {
                            normalize(value, names);
                        }
                    }
                }
                serde_json::Value::Array(items) => {
                    for value in items {
                        normalize(value, names);
                    }
                }
                _ => {}
            }
        }
        let mut value = serde_json::to_value(routine).map_err(|e| e.to_string())?;
        // Host crates may enable serde_json's preserve_order feature. Object
        // insertion order must not create a new version of the same routine.
        value.sort_all_objects();
        normalize(&mut value, &mut BTreeMap::new());
        let bytes = serde_json::to_vec(&value).map_err(|e| e.to_string())?;
        Ok(format!("v1-{:x}", Sha256::digest(bytes)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Check, Step};
    use serde_json::json;

    fn routine() -> Routine {
        Routine {
            steps: (0..2)
                .map(|_| Step {
                    tool: "read".into(),
                    arguments: json!({"path":{"$input":"path"}}),
                    check: Check {
                        pointer: "/ok".into(),
                        equals: json!(true),
                    },
                })
                .collect(),
        }
    }

    fn activate(library: &mut Library) -> String {
        library
            .admit(routine(), &uuid::Uuid::new_v4().to_string())
            .unwrap();
        library
            .admit(routine(), &uuid::Uuid::new_v4().to_string())
            .unwrap()
    }

    #[test]
    fn persisted_in_flight_work_is_retired_after_restart() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("routines.json");
        let mut library = Library::open(&path).unwrap();
        let id = activate(&mut library);
        library.begin(&id).unwrap();
        assert!(library.begin(&id).is_err());
        assert!(Library::open(&path).is_err());
        drop(library);
        let restored = Library::open(&path).unwrap();
        assert!(restored.catalog()[&id].retired);
        assert!(restored.resolve(&id).is_err());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
    }

    #[test]
    fn atomic_store_retains_outcomes_and_fails_closed_on_write_failure() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("routines.json");
        let mut library = Library::open(&path).unwrap();
        let id = activate(&mut library);
        library.begin(&id).unwrap();
        library.outcome(&id, true).unwrap();
        assert_eq!(
            Library::restore(&std::fs::read(&path).unwrap())
                .unwrap()
                .catalog()[&id]
                .successes,
            3
        );
        library.begin(&id).unwrap();
        library.outcome(&id, false).unwrap();
        library.begin(&id).unwrap();
        library.outcome(&id, false).unwrap();
        assert!(
            Library::restore(&std::fs::read(&path).unwrap())
                .unwrap()
                .catalog()[&id]
                .retired
        );
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 2);
        drop(library);
        // Turn the parent into a missing directory without relying on Unix modes.
        std::fs::remove_file(&path).unwrap();
        std::fs::remove_file(path.with_extension("lock")).unwrap();
        std::fs::remove_dir(dir.path()).unwrap();
        let mut fresh = Library::default();
        let fresh_id = activate(&mut fresh);
        fresh.path = Some(path);
        assert!(fresh.begin(&fresh_id).is_err());
        assert!(fresh.resolve(&fresh_id).is_err());
        assert!(
            fresh
                .admit(routine(), &uuid::Uuid::new_v4().to_string())
                .is_err()
        );
    }

    #[test]
    fn corrupt_unbounded_or_mismatched_snapshots_are_rejected() {
        assert!(Library::restore(b"garbage").is_err());
        assert!(Library::restore(&vec![0; 3_000_001]).is_err());
        assert!(Library::restore(br#"{"version":9,"records":{}}"#).is_err());
        let mut library = Library::default();
        let id = activate(&mut library);
        let mut snapshot: serde_json::Value =
            serde_json::from_slice(&library.snapshot().unwrap()).unwrap();
        snapshot["records"][&id]["routine"]["steps"][0]["tool"] = json!("different");
        assert!(Library::restore(&serde_json::to_vec(&snapshot).unwrap()).is_err());
        let dir = tempfile::tempdir().unwrap();
        assert!(Library::open(dir.path()).is_err());
    }

    #[test]
    fn bounded_library_preserves_retirement_entries() {
        let mut library = Library::default();
        for index in 0..128 {
            let mut r = routine();
            r.steps[0].tool = format!("read_{index}");
            library.admit(r, &uuid::Uuid::new_v4().to_string()).unwrap();
        }
        assert!(
            library
                .admit(routine(), &uuid::Uuid::new_v4().to_string())
                .is_err()
        );
    }
}
