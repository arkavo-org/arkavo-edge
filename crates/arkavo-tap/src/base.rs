//! The RFC 9421 signature base (section 2.5).
//!
//! One line per covered component, then `@signature-params`. Built only from the request
//! context and the received parameter serialization, never by concatenating an
//! application's reconstructed URL.

use crate::context::RequestContext;
use crate::error::Rejection;

/// The exact bytes a signature covers.
pub fn signature_base(
    ctx: &RequestContext,
    components: &[String],
    signature_params: &str,
) -> Result<Vec<u8>, Rejection> {
    let mut base = String::new();
    for name in components {
        if name == "@signature-params" {
            return Err(Rejection::malformed(
                "@signature-params cannot be a covered component",
            ));
        }
        let value = ctx.component(name)?;
        base.push('"');
        base.push_str(name);
        base.push_str("\": ");
        base.push_str(&value);
        base.push('\n');
    }
    base.push_str("\"@signature-params\": ");
    base.push_str(signature_params);
    Ok(base.into_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::ExternalOrigin;

    /// The test-request of RFC 9421 appendix B.2.
    fn rfc_request() -> RequestContext {
        RequestContext::new(
            ExternalOrigin::parse("https://example.com").unwrap(),
            "POST",
            "/foo?param=Value&Pet=dog",
            vec![
                ("Host".into(), "example.com".into()),
                ("Date".into(), "Tue, 20 Apr 2021 02:07:55 GMT".into()),
                ("Content-Type".into(), "application/json".into()),
                (
                    "Content-Digest".into(),
                    "sha-512=:WZDPaVn/7XgHaAy8pmojAkGWoRx2UFChF41A2svX+TaPm+AbwAgBWnrIiYllu7BNNyealdVLvRwEmTHWXvJwew==:".into(),
                ),
                ("Content-Length".into(), "18".into()),
            ],
        )
        .unwrap()
    }

    #[test]
    fn reproduces_rfc_9421_b_2_6() {
        let components: Vec<String> = [
            "date",
            "@method",
            "@path",
            "@authority",
            "content-type",
            "content-length",
        ]
        .map(String::from)
        .to_vec();
        let params = r#"("date" "@method" "@path" "@authority" "content-type" "content-length");created=1618884473;keyid="test-key-ed25519""#;
        let base = signature_base(&rfc_request(), &components, params).unwrap();
        let expected = concat!(
            "\"date\": Tue, 20 Apr 2021 02:07:55 GMT\n",
            "\"@method\": POST\n",
            "\"@path\": /foo\n",
            "\"@authority\": example.com\n",
            "\"content-type\": application/json\n",
            "\"content-length\": 18\n",
            "\"@signature-params\": (\"date\" \"@method\" \"@path\" \"@authority\" \"content-type\" \"content-length\");created=1618884473;keyid=\"test-key-ed25519\"",
        );
        assert_eq!(String::from_utf8(base).unwrap(), expected);
    }

    #[test]
    fn refuses_signature_params_and_absent_fields() {
        let ctx = rfc_request();
        assert!(signature_base(&ctx, &["@signature-params".into()], "()").is_err());
        assert!(signature_base(&ctx, &["idempotency-key".into()], "()").is_err());
    }
}
