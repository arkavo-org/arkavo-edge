//! Asking the Arkavo market for a purchase on the owner's behalf.
//!
//! The market (`market.arkavo.com`) sells prepaid terms through *requests*:
//! an agent its owner has delegated to asks for a term, and the market
//! answers with a payment link. The owner, or anyone the owner shares the
//! link with, pays it. The agent never pays and holds no payment credential.
//!
//! Contract: arkavo-com/market `docs/market-identity-interface.md`
//! ("Requests: the tokens identity issues", "Proof of possession"), published
//! at <https://market.arkavo.com/api>.

mod client;
mod error;
mod tool;

pub use client::{MARKET_URL, MarketClient, MarketRequest, REQUEST_ENTITLEMENT, Term, Withdrawal};
pub use error::MarketError;
pub use tool::{AgentKey, MARKET_REQUEST_TOOL, MarketRequestTool, register_tools};
