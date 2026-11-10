//! `arkavo market`: ask the Arkavo market for a purchase the owner pays.

use arkavo_market::{MarketClient, MarketRequest, Term, Withdrawal};

pub(crate) fn market_help() -> &'static str {
    "    market         Ask the Arkavo market for a purchase the owner pays"
}

const HELP: &str = "Ask the Arkavo market for a purchase the owner pays

USAGE:
    arkavo market request --offer <OFFER_ID> --term <3|12>
    arkavo market withdraw <REQUEST_ID>

COMMANDS:
    request     Ask for a prepaid term, in months, of an offer. Prints a payment
                link: the owner, or anyone they share it with, pays it. Asking
                again for the same term prints the same link; a different term
                replaces it, and the earlier link stops working
    withdraw    Withdraw an open request; its link stops working

The request is made as this machine's agent, for the account of the owner who
authorized it. The owner must have allowed purchase requests: start the agent
with `arkavo agent run --market-requests` and approve the link it shows.
Offer IDs are listed at https://market.arkavo.com/api.

OPTIONS:
    -h, --help    Show this help";

/// What an invocation asks for, decided from its arguments alone.
#[derive(Debug, PartialEq, Eq)]
enum Invocation {
    Help,
    Request { offer: String, term: Term },
    Withdraw { request_id: String },
}

fn refuse(message: &str) -> String {
    format!("{message}\nRun 'arkavo market --help' for usage")
}

/// Decide what to do before anything is asked of identity or the market.
fn plan(args: &[String]) -> Result<Invocation, String> {
    if args.is_empty()
        || args
            .iter()
            .any(|a| matches!(a.as_str(), "-h" | "--help" | "help"))
    {
        return Ok(Invocation::Help);
    }
    match args[0].as_str() {
        "request" => plan_request(&args[1..]),
        "withdraw" => match &args[1..] {
            [id] if !id.starts_with('-') => Ok(Invocation::Withdraw {
                request_id: id.clone(),
            }),
            _ => Err(refuse("'arkavo market withdraw' takes one request ID")),
        },
        other => Err(refuse(&format!("unknown market command '{other}'"))),
    }
}

fn plan_request(args: &[String]) -> Result<Invocation, String> {
    let mut offer = None;
    let mut term = None;
    let mut rest = args.iter();
    while let Some(arg) = rest.next() {
        let value = rest.next().filter(|value| !value.starts_with('-'));
        match (arg.as_str(), value) {
            ("--offer", Some(value)) => offer = Some(value.clone()),
            ("--term", Some(value)) => {
                let months = value
                    .parse::<u64>()
                    .map_err(|_| refuse("--term is 3 or 12, in months"))?;
                term = Some(Term::try_from(months).map_err(|e| refuse(&e.to_string()))?);
            }
            ("--offer" | "--term", None) => {
                return Err(refuse(&format!("{arg} takes a value")));
            }
            (other, _) => return Err(refuse(&format!("unexpected argument '{other}'"))),
        }
    }
    match (offer, term) {
        (Some(offer), Some(term)) => Ok(Invocation::Request { offer, term }),
        _ => Err(refuse("'arkavo market request' needs --offer and --term")),
    }
}

fn describe(request: &MarketRequest) -> String {
    format!(
        "Request {id} is {status}: {term} months.\n\nPayment link:\n    {url}\n\nThe owner, or anyone \
         they share the link with, pays it. This agent cannot. Once it is paid, the purchase is \
         on the owner's account within a few minutes.",
        id = request.request,
        status = request.status,
        term = request.term,
        url = request.url,
    )
}

// Both `Handle::block_on` and `Runtime::block_on` are disallowed by
// `.clippy.toml` (nesting a runtime inside a runtime can panic). This is a
// CLI entry point: it reuses an ambient runtime when one exists and only
// builds its own otherwise.
#[allow(clippy::disallowed_methods)]
pub fn execute(args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    let invocation = plan(args)?;
    if invocation == Invocation::Help {
        println!("{HELP}");
        return Ok(());
    }
    let run_async = run(invocation);
    match tokio::runtime::Handle::try_current() {
        Ok(handle) => handle.block_on(run_async),
        Err(_) => tokio::runtime::Runtime::new()?.block_on(run_async),
    }
}

async fn run(invocation: Invocation) -> Result<(), Box<dyn std::error::Error>> {
    let client = MarketClient::new()?;
    let keypair = arkavo_registration::load_or_create_agent_keypair()?;
    match invocation {
        Invocation::Help => {}
        Invocation::Request { offer, term } => {
            let request = client.ask(&keypair, &offer, term).await?;
            println!("{}", describe(&request));
        }
        Invocation::Withdraw { request_id } => {
            match client.withdraw(&keypair, &request_id).await? {
                Withdrawal::Withdrawn => {
                    println!("Withdrew {request_id}. Its payment link no longer works.");
                }
                Withdrawal::NotOpen => println!(
                    "No open request {request_id} for this account: it was already paid, \
                     withdrawn or replaced."
                ),
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(values: &[&str]) -> Vec<String> {
        values.iter().map(ToString::to_string).collect()
    }

    #[test]
    fn a_request_names_an_offer_and_a_term() {
        assert_eq!(
            plan(&args(&[
                "request",
                "--offer",
                "arkavo-online-play",
                "--term",
                "12"
            ])),
            Ok(Invocation::Request {
                offer: "arkavo-online-play".into(),
                term: Term::Twelve,
            })
        );
        assert_eq!(
            plan(&args(&["request", "--term", "3", "--offer", "o"])),
            Ok(Invocation::Request {
                offer: "o".into(),
                term: Term::Three,
            })
        );
    }

    /// A request creates a payment link in the owner's name, so nothing
    /// about it is guessed: a missing or wrong argument is refused.
    #[test]
    fn a_request_missing_or_misnaming_an_argument_is_refused() {
        for values in [
            &["request"][..],
            &["request", "--offer", "o"],
            &["request", "--term", "3"],
            &["request", "--offer", "o", "--term", "6"],
            &["request", "--offer", "o", "--term", "three"],
            &["request", "--offer", "--term", "3"],
            &[
                "request",
                "--offer",
                "o",
                "--term",
                "3",
                "--url",
                "https://example.com",
            ],
            &["request", "--offer", "o", "--term", "3", "extra"],
        ] {
            let err = plan(&args(values)).unwrap_err();
            assert!(err.contains("arkavo market --help"), "{values:?}: {err}");
        }
    }

    #[test]
    fn a_withdrawal_names_one_request() {
        assert_eq!(
            plan(&args(&["withdraw", "rq_AAAA"])),
            Ok(Invocation::Withdraw {
                request_id: "rq_AAAA".into()
            })
        );
        for values in [
            &["withdraw"][..],
            &["withdraw", "rq_a", "rq_b"],
            &["withdraw", "--all"],
        ] {
            assert!(plan(&args(values)).is_err(), "{values:?}");
        }
    }

    #[test]
    fn help_is_asked_for_or_shown_when_nothing_is() {
        for values in [
            &[][..],
            &["--help"],
            &["-h"],
            &["help"],
            &["request", "--help"],
        ] {
            assert_eq!(plan(&args(values)), Ok(Invocation::Help), "{values:?}");
        }
        assert!(plan(&args(&["buy"])).is_err());
    }

    #[test]
    fn the_answer_shows_the_link_and_who_pays() {
        let text = describe(&MarketRequest {
            request: "rq_AAAA".into(),
            url: "https://buy.stripe.com/test_abc".into(),
            term: 3,
            status: "open".into(),
        });
        assert!(text.contains("https://buy.stripe.com/test_abc"), "{text}");
        assert!(text.contains("rq_AAAA"), "{text}");
        assert!(text.contains("3 months"), "{text}");
    }

    #[test]
    fn the_help_names_both_subcommands() {
        assert!(HELP.contains("request"));
        assert!(HELP.contains("withdraw"));
        assert!(market_help().contains("market"));
    }
}
