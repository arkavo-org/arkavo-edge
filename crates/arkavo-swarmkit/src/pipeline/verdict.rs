//! The line a critic role ends its review with.
//!
//! A pipeline cannot read a rubric score out of prose, so the critic states
//! its decision on a line of its own. The reading is strict on purpose: a
//! review that does not say `PASS` in exactly the agreed form has not passed,
//! which keeps a truncated or off-format review from letting work through.

/// The line that passes the work under review.
pub const VERDICT_PASS_LINE: &str = "VERDICT: PASS";

/// The line that sends the work back.
pub const VERDICT_FAIL_LINE: &str = "VERDICT: FAIL";

/// What a critic's output says about the work it reviewed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    Pass,
    Fail,
    /// No verdict line at all. Counts as a failing verdict; kept apart so
    /// the caller can be told the critic never gave one.
    Missing,
}

impl Verdict {
    /// True only for an explicit `VERDICT: PASS`.
    pub const fn passed(self) -> bool {
        matches!(self, Self::Pass)
    }

    /// How the verdict is named in messages and results.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Pass => "PASS",
            Self::Fail => "FAIL",
            Self::Missing => "missing",
        }
    }
}

/// Read the verdict out of a critic's output.
///
/// A verdict line is a line that, with surrounding whitespace removed, is
/// exactly [`VERDICT_PASS_LINE`] or [`VERDICT_FAIL_LINE`]. Case matters and
/// nothing else may share the line. When several lines qualify the last one
/// wins, so a critic that quotes the instruction and then decides is read by
/// its decision.
pub fn parse_verdict(output: &str) -> Verdict {
    output
        .lines()
        .rev()
        .find_map(|line| match line.trim() {
            VERDICT_PASS_LINE => Some(Verdict::Pass),
            VERDICT_FAIL_LINE => Some(Verdict::Fail),
            _ => None,
        })
        .unwrap_or(Verdict::Missing)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_pass_line_passes() {
        assert_eq!(parse_verdict("Looks right.\nVERDICT: PASS"), Verdict::Pass);
        assert!(parse_verdict("VERDICT: PASS").passed());
    }

    #[test]
    fn a_fail_line_fails() {
        assert_eq!(
            parse_verdict("Two claims are unsupported.\nVERDICT: FAIL\n"),
            Verdict::Fail
        );
        assert!(!Verdict::Fail.passed());
    }

    #[test]
    fn surrounding_whitespace_is_allowed() {
        assert_eq!(parse_verdict("  \tVERDICT: PASS  \r\n"), Verdict::Pass);
        assert_eq!(parse_verdict("notes\r\n VERDICT: FAIL\r\n"), Verdict::Fail);
    }

    #[test]
    fn the_last_verdict_line_wins() {
        assert_eq!(
            parse_verdict("VERDICT: PASS\nOn reflection:\nVERDICT: FAIL"),
            Verdict::Fail
        );
        assert_eq!(
            parse_verdict("VERDICT: FAIL\nAfter the fix:\nVERDICT: PASS\nThanks."),
            Verdict::Pass
        );
    }

    /// The pipeline fails closed: work passes only on an explicit PASS.
    #[test]
    fn no_verdict_line_is_missing_and_does_not_pass() {
        for output in ["", "The copy reads well.", "\n\n"] {
            let verdict = parse_verdict(output);
            assert_eq!(verdict, Verdict::Missing, "{output:?}");
            assert!(!verdict.passed(), "{output:?}");
        }
    }

    #[test]
    fn case_matters() {
        for output in ["verdict: pass", "Verdict: Pass", "VERDICT: pass"] {
            assert_eq!(parse_verdict(output), Verdict::Missing, "{output:?}");
        }
    }

    #[test]
    fn nothing_else_may_share_the_line() {
        for output in [
            "VERDICT: PASS.",
            "**VERDICT: PASS**",
            "VERDICT: PASS with reservations",
            "Final VERDICT: PASS",
            "VERDICT:PASS",
            "VERDICT:  PASS",
            "> VERDICT: PASS",
        ] {
            assert_eq!(parse_verdict(output), Verdict::Missing, "{output:?}");
        }
    }

    #[test]
    fn a_decorated_pass_does_not_override_a_plain_fail() {
        assert_eq!(
            parse_verdict("VERDICT: FAIL\n**VERDICT: PASS**"),
            Verdict::Fail
        );
    }

    #[test]
    fn verdicts_are_named_for_messages() {
        assert_eq!(Verdict::Pass.as_str(), "PASS");
        assert_eq!(Verdict::Fail.as_str(), "FAIL");
        assert_eq!(Verdict::Missing.as_str(), "missing");
    }
}
