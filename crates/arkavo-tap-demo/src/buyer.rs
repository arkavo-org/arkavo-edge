//! The buyer: a test agent that signs its requests to the merchant.
//!
//! Requests are built completely (URL, body, `Content-Digest`, `Idempotency-Key`) before
//! they are signed, then sent exactly as signed. The signer only signs for the configured
//! merchant origin.

use crate::config::DemoConfig;
use crate::keyfile;
use crate::server::SharedClock;
use anyhow::Context;
use arkavo_tap::signer::{prepare, sign};
use arkavo_tap::{
    Ed25519Signer, ExternalOrigin, Operation, Profile, Purpose, RequestContext, SignError,
    SigningPolicy, TapSigner as _,
};
use reqwest::Method;

/// A request as it will go on the wire.
#[derive(Debug, Clone)]
pub struct Prepared {
    pub method: String,
    pub path: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

/// A request before it is signed.
#[derive(Debug, Clone, Copy)]
pub struct Draft<'a> {
    pub method: &'a str,
    pub path: &'a str,
    pub body: Option<&'a [u8]>,
    pub operation: Operation,
    pub purpose: Purpose,
    pub idempotency_key: Option<&'a str>,
}

impl<'a> Draft<'a> {
    pub fn read(path: &'a str, purpose: Purpose) -> Self {
        Draft {
            method: "GET",
            path,
            body: None,
            operation: Operation::Read,
            purpose,
            idempotency_key: None,
        }
    }

    pub fn write(
        path: &'a str,
        body: &'a [u8],
        operation: Operation,
        idempotency_key: &'a str,
    ) -> Self {
        Draft {
            method: "POST",
            path,
            body: Some(body),
            operation,
            purpose: Purpose::Pay,
            idempotency_key: Some(idempotency_key),
        }
    }
}

/// A response: its status and JSON body (null when the body is not JSON).
#[derive(Debug, Clone)]
pub struct Answer {
    pub status: u16,
    pub body: serde_json::Value,
}

pub struct Buyer {
    client: reqwest::Client,
    signer: Ed25519Signer,
    origin: ExternalOrigin,
    profile: Profile,
    lifetime: i64,
    clock: SharedClock,
}

impl Buyer {
    /// The agent whose key is at `key_path`, registered as `key_id`.
    pub fn from_config(
        config: &DemoConfig,
        key_path: &str,
        key_id: &str,
        clock: SharedClock,
    ) -> anyhow::Result<Self> {
        let key = keyfile::load(&config.path(key_path))?;
        let origin = config.origin()?;
        let signer = Ed25519Signer::new(
            &key.to_bytes(),
            key_id,
            SigningPolicy {
                destinations: vec![origin.as_origin()],
                operations: vec![Operation::Read, Operation::Checkout, Operation::Commit],
            },
        );
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .no_proxy()
            .build()?;
        Ok(Buyer {
            client,
            signer,
            origin,
            profile: config.profile()?,
            lifetime: config.signature.outbound_lifetime_seconds,
            clock,
        })
    }

    pub fn key_id(&self) -> &str {
        self.signer.key_id()
    }

    pub fn public_key(&self) -> [u8; 32] {
        self.signer.public_key()
    }

    /// Builds and signs a request on the merchant.
    pub fn prepare(&self, draft: &Draft<'_>) -> Result<Prepared, SignError> {
        self.prepare_for(&self.origin, draft)
    }

    /// As [`Buyer::prepare`], for any origin: the signer's own policy decides.
    pub fn prepare_for(
        &self,
        origin: &ExternalOrigin,
        draft: &Draft<'_>,
    ) -> Result<Prepared, SignError> {
        let mut headers = Vec::new();
        if let Some(body) = draft.body {
            headers.push(("content-type".to_owned(), "application/json".to_owned()));
            headers.push((
                "content-digest".to_owned(),
                arkavo_tap::digest::content_digest(body),
            ));
        }
        if let Some(key) = draft.idempotency_key {
            headers.push(("idempotency-key".to_owned(), key.to_owned()));
        }
        let ctx = RequestContext::new(origin.clone(), draft.method, draft.path, headers.clone())
            .map_err(|e| SignError::InvalidRequest(e.detail))?;
        let op = prepare(
            ctx,
            self.profile,
            draft.operation,
            draft.purpose,
            self.key_id(),
            self.clock.now(),
            self.lifetime,
        )?;
        let signed = sign(&self.signer, &op)?;
        headers.push(("signature-input".to_owned(), signed.signature_input));
        headers.push(("signature".to_owned(), signed.signature));
        Ok(Prepared {
            method: draft.method.to_owned(),
            path: draft.path.to_owned(),
            headers,
            body: draft.body.map(<[u8]>::to_vec).unwrap_or_default(),
        })
    }

    /// Sends a prepared request as it is.
    pub async fn dispatch(&self, request: &Prepared) -> anyhow::Result<Answer> {
        let method = Method::from_bytes(request.method.as_bytes())?;
        let url = format!("{}{}", self.origin.as_origin(), request.path);
        let mut builder = self.client.request(method, url).body(request.body.clone());
        for (name, value) in &request.headers {
            builder = builder.header(name, value);
        }
        let response = builder.send().await.context("sending to the merchant")?;
        let status = response.status().as_u16();
        let body = response.json().await.unwrap_or(serde_json::Value::Null);
        Ok(Answer { status, body })
    }

    /// Signs and sends.
    pub async fn send(&self, draft: &Draft<'_>) -> anyhow::Result<Answer> {
        let prepared = self.prepare(draft)?;
        self.dispatch(&prepared).await
    }

    pub async fn browse(&self, product: &str) -> anyhow::Result<Answer> {
        let path = format!("/demo/v1/products/{product}");
        self.send(&Draft::read(&path, Purpose::Browse)).await
    }

    pub async fn checkout(
        &self,
        items: &[(&str, u32)],
        idempotency_key: &str,
    ) -> anyhow::Result<Answer> {
        let items: Vec<_> = items
            .iter()
            .map(|(id, q)| serde_json::json!({ "id": id, "quantity": q }))
            .collect();
        let body = serde_json::to_vec(&serde_json::json!({ "items": items }))?;
        self.send(&Draft::write(
            "/demo/v1/checkouts",
            &body,
            Operation::Checkout,
            idempotency_key,
        ))
        .await
    }

    pub fn order_body(
        checkout_id: &str,
        grant: Option<&arkavo_tap::SignedEnvelope>,
    ) -> anyhow::Result<Vec<u8>> {
        let mut body = serde_json::json!({ "checkoutId": checkout_id });
        if let Some(grant) = grant {
            body["grant"] = serde_json::to_value(grant)?;
        }
        Ok(serde_json::to_vec(&body)?)
    }

    pub async fn order(
        &self,
        checkout_id: &str,
        grant: Option<&arkavo_tap::SignedEnvelope>,
        idempotency_key: &str,
    ) -> anyhow::Result<Answer> {
        let body = Self::order_body(checkout_id, grant)?;
        self.send(&Draft::write(
            "/demo/v1/orders",
            &body,
            Operation::Commit,
            idempotency_key,
        ))
        .await
    }

    pub async fn order_status(&self, order_id: &str) -> anyhow::Result<Answer> {
        let path = format!("/demo/v1/orders/{order_id}");
        self.send(&Draft::read(&path, Purpose::Pay)).await
    }

    /// An unsigned request, for anonymous routes and negative tests.
    pub async fn unsigned(
        &self,
        method: &str,
        path: &str,
        headers: &[(&str, &str)],
    ) -> anyhow::Result<Answer> {
        let prepared = Prepared {
            method: method.to_owned(),
            path: path.to_owned(),
            headers: headers
                .iter()
                .map(|(n, v)| ((*n).to_owned(), (*v).to_owned()))
                .collect(),
            body: Vec::new(),
        };
        self.dispatch(&prepared).await
    }
}
