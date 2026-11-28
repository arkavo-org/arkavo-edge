//! Workload keys and the key statements they sign, for tests.

use arkavo_crypto::{AgentPublicKey, P256SigningKeypair};
use arkavo_key_statement::{KeyStatement, Purpose};

/// The time tests check statements at, in Unix seconds.
pub(crate) const NOW: u64 = 1_800_000_000;

pub(crate) struct Workload {
    keypair: P256SigningKeypair,
    pub(crate) did: String,
}

impl Workload {
    pub(crate) fn new() -> Self {
        let keypair = P256SigningKeypair::generate();
        let did = keypair.public_key().to_did_key();
        Self { keypair, did }
    }

    /// A statement vouching for `key` as this agent's gossip key, valid
    /// around [`NOW`].
    pub(crate) fn vouch(&self, key: &AgentPublicKey, generation: u64) -> (String, KeyStatement) {
        self.sign(KeyStatement {
            issuer: self.did.clone(),
            key: key.to_did_key(),
            purpose: Purpose::Gossip,
            generation,
            not_before: NOW - 60,
            not_after: NOW + 3600,
        })
    }

    pub(crate) fn sign(&self, statement: KeyStatement) -> (String, KeyStatement) {
        let key_id = format!("{}#{}", self.did, &self.did["did:key:".len()..]);
        let compact = arkavo_key_statement::issue(&statement, &key_id, |input| {
            Ok::<_, String>(self.keypair.sign_p1363(input))
        })
        .expect("a well-formed statement signs");
        (compact, statement)
    }
}
