use arkavo_agent_auth::AgentAuthError;
use thiserror::Error;

/// Why an ask or a withdrawal did not go through, in words a person can act
/// on: the market's own refusals carry one word and, for a 401, no reason.
#[derive(Error, Debug)]
pub enum MarketError {
    #[error(
        "This agent's owner has not let it ask the market for purchases. The owner runs \
         `arkavo agent authorize --market-requests` on this machine and approves it in Arkavo \
         Creator."
    )]
    NotDelegated,

    #[error("{0}")]
    Identity(#[from] AgentAuthError),

    #[error("{0}")]
    InvalidRequest(String),

    #[error("The market refused the request as malformed. Check the offer ID and the term.")]
    BadRequest,

    #[error("The market refused this agent's credentials and does not say why. {0}")]
    Unauthorized(String),

    #[error(
        "The market has no offer `{0}`. Offer IDs are listed at https://market.arkavo.com/api."
    )]
    UnknownOffer(String),

    #[error(
        "Another request for this account changed at the same moment. Nothing was lost; ask again."
    )]
    Conflict,

    #[error(
        "The market makes one new payment link per account every 30 seconds and is still \
         holding this account back. Ask again in {retry_after} seconds."
    )]
    TooManyRequests { retry_after: u64 },

    #[error("The market is unavailable right now. Nothing was requested; try again later.")]
    Unavailable,

    #[error("The market answered with an unexpected status {0}.")]
    Unexpected(u16),

    #[error("The market's answer could not be used: {0}")]
    InvalidResponse(String),

    #[error("The market address is not usable: {0}")]
    InvalidEndpoint(String),

    #[error("Could not reach the market: {0}")]
    Http(#[from] reqwest::Error),
}
