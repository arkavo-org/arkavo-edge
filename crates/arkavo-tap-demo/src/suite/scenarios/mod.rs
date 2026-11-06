//! The acceptance scenarios, by ID. Each returns why it failed, or nothing.

mod purchases;
mod signatures;

use super::{Coverage, Env, Line};
use crate::buyer::{Answer, Buyer, Draft, Prepared};
use crate::merchant::RECEIPT;
use arkavo_tap::{
    Clock as _, Ed25519Signer, ExternalOrigin, ManualClock, Operation, Purpose, SignError,
    SignedEnvelope, SigningPolicy,
};
use std::sync::Arc;

type Check = Result<(), String>;

pub(super) fn ensure(condition: bool, why: impl Into<String>) -> Check {
    if condition { Ok(()) } else { Err(why.into()) }
}

pub(super) fn got(answer: anyhow::Result<Answer>) -> Result<Answer, String> {
    answer.map_err(|e| format!("request failed: {e:#}"))
}

pub(super) fn expect(answer: &Answer, status: u16, code: Option<&str>) -> Check {
    ensure(
        answer.status == status,
        format!("expected {status}, got {} {}", answer.status, answer.body),
    )?;
    match code {
        Some(code) => ensure(
            answer.body["error"] == code,
            format!("expected {code}, got {}", answer.body),
        ),
        None => Ok(()),
    }
}

pub(super) fn str_of<'a>(v: &'a serde_json::Value, field: &str) -> Result<&'a str, String> {
    v[field]
        .as_str()
        .ok_or_else(|| format!("no {field} in {v}"))
}

/// Creates a checkout for `items` and returns its ID and hash.
pub(super) async fn checkout(
    buyer: &Buyer,
    items: &[(&str, u32)],
    key: &str,
) -> Result<(String, String), String> {
    let answer = got(buyer.checkout(items, key).await)?;
    expect(&answer, 201, None)?;
    Ok((
        str_of(&answer.body["checkout"], "checkoutId")?.to_owned(),
        str_of(&answer.body, "checkoutHash")?.to_owned(),
    ))
}

macro_rules! ran {
    ($id:literal, $title:literal, $f:path, $env:expr) => {
        Line {
            id: $id,
            title: $title,
            coverage: Coverage::Ran($f($env).await),
        }
    };
}

/// Every scenario, in an order that keeps them independent: the ones that revoke keys and
/// stop the registry run last.
pub(super) async fn all(env: &Env) -> Vec<Line> {
    let elsewhere = |id, title, at| Line {
        id,
        title,
        coverage: Coverage::Elsewhere(at),
    };
    vec![
        ran!("T01", "valid signed product request", signatures::t01, env),
        ran!(
            "T02",
            "missing signature on a protected route",
            signatures::t02,
            env
        ),
        ran!("T03", "modified authority or path", signatures::t03, env),
        ran!("T04", "modified method or query", signatures::t04, env),
        ran!(
            "T05",
            "modified content under the original digest",
            signatures::t05,
            env
        ),
        ran!(
            "T06",
            "valid digest outside the required coverage",
            signatures::t06,
            env
        ),
        ran!("T07", "duplicate signature parameter", signatures::t07, env),
        ran!("T08", "unknown key", signatures::t08, env),
        ran!("T09", "future and expired signatures", signatures::t09, env),
        ran!("T10", "the same signed request twice", signatures::t10, env),
        elsewhere(
            "T11",
            "concurrent replay across two verifiers",
            "arkavo-tap acceptance t11 and replay tests",
        ),
        elsewhere(
            "T12",
            "replay after a verifier restart",
            "arkavo-tap acceptance t10_t12",
        ),
        ran!("T15", "forged verified-agent headers", signatures::t15, env),
        Line {
            id: "T16",
            title: "consumer recognition evidence",
            coverage: Coverage::Disabled(
                "no canonical serialization upstream; the principal stays unresolved",
            ),
        },
        ran!(
            "T17",
            "agent B presents agent A's grant",
            purchases::t17,
            env
        ),
        ran!(
            "T18",
            "absent, expired or revoked grant",
            purchases::t18,
            env
        ),
        ran!(
            "T19",
            "altered merchant, item, quantity, currency or total",
            purchases::t19,
            env
        ),
        ran!(
            "T20",
            "retry with a fresh signature and the same key",
            purchases::t20,
            env
        ),
        ran!(
            "T21",
            "the same idempotency key for another checkout",
            purchases::t21,
            env
        ),
        ran!(
            "T22",
            "crash after submission, reconciled",
            purchases::t22,
            env
        ),
        ran!(
            "T23",
            "simulated outcome is never a settlement",
            purchases::t23,
            env
        ),
        ran!(
            "T24",
            "development grant presented as production",
            purchases::t24,
            env
        ),
        elsewhere(
            "T25",
            "independent implementation in both directions",
            "arkavo-tap acceptance interop (RFC 9421 B.2.6, httpsig)",
        ),
        ran!(
            "T26",
            "signing or ordering outside policy",
            purchases::t26,
            env
        ),
        Line {
            id: "T27",
            title: "AP2 transaction without a mandate",
            coverage: Coverage::Disabled(
                "AP2 is not enabled; configuration refuses mode = \"ap2\"",
            ),
        },
        ran!(
            "T28",
            "no secrets in refusals or debug output",
            purchases::t28,
            env
        ),
        ran!(
            "T29",
            "approved simulated purchase receipt",
            purchases::t29,
            env
        ),
        ran!("T30", "unsigned public catalog", purchases::t30, env),
        ran!("T14", "key revoked while cached", signatures::t14, env),
        ran!("T13", "registry unavailable", signatures::t13, env),
    ]
}
