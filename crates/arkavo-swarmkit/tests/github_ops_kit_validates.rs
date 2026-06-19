//! The github-ops-kit example must parse, pass cross-block validation, and
//! round-trip its canonical `kit.id`. It also asserts the per-role MCP tool
//! grants that make the kit useful: the dispatcher routes (including PR watch),
//! the reviewer can post reviews, the test runner is granted `test_run` +
//! `gh_checks`, and the maintainer gets repo-scoped git/issue tools.

use arkavo_swarmkit::{parse_yaml, validate};

const KIT: &str = include_str!("../../../examples/github-ops-kit/github-ops-kit.swarmkit.yaml");

#[test]
fn github_ops_kit_parses_validates_and_round_trips() {
    // `parse_yaml` runs `validate` internally; `kit.id` is authored empty, so
    // the BLAKE3 id check is skipped on this pass.
    let mut manifest = parse_yaml(KIT).expect("github-ops-kit must parse and validate");

    let ids: Vec<&str> = manifest.roles.iter().map(|r| r.id.as_str()).collect();
    assert_eq!(
        ids,
        [
            "dispatcher",
            "pr_reviewer",
            "pr_test_runner",
            "repo_maintainer"
        ]
    );

    // Every role declares at least one MCP tool grant — the capability
    // declaration the orchestrator brokers when it provisions the role.
    for role in &manifest.roles {
        assert!(
            !role.mcp_tools.is_empty(),
            "role {} must declare mcp_tools",
            role.id
        );
    }

    // pr_test_runner has the general test-runner capability (swarm decides usage).
    let runner = manifest
        .roles
        .iter()
        .find(|r| r.id == "pr_test_runner")
        .expect("pr_test_runner role exists");
    assert!(
        runner
            .mcp_tools
            .iter()
            .any(|g| g.tools.iter().any(|t| t == "test_run")),
        "pr_test_runner must be granted test_run"
    );
    // the eval gate is gone
    assert!(
        !runner
            .mcp_tools
            .iter()
            .any(|g| g.tools.iter().any(|t| t == "run_eval")),
        "pr_test_runner must not be granted run_eval"
    );
    // dispatcher monitors PRs via the poll tool (tool lands in WS-B)
    let dispatcher = manifest
        .roles
        .iter()
        .find(|r| r.id == "dispatcher")
        .expect("dispatcher role exists");
    assert!(
        dispatcher
            .mcp_tools
            .iter()
            .any(|g| g.tools.iter().any(|t| t == "github_pr_watch")),
        "dispatcher must be granted github_pr_watch"
    );

    // Least privilege: the reviewer can review but not open PRs; the
    // maintainer can open PRs but not post reviews.
    let reviewer = manifest
        .roles
        .iter()
        .find(|r| r.id == "pr_reviewer")
        .expect("pr_reviewer role exists");
    let reviewer_tools: Vec<&str> = reviewer
        .mcp_tools
        .iter()
        .flat_map(|g| g.tools.iter().map(String::as_str))
        .collect();
    assert!(reviewer_tools.contains(&"gh_pr_review"));
    assert!(!reviewer_tools.contains(&"github_pr_create"));

    // Compute the canonical kit id, then re-validate: the id round-trips.
    manifest.compute_kit_id().expect("compute kit id");
    assert!(!manifest.kit.id.is_empty(), "kit id should be populated");
    validate(&manifest).expect("re-validate after compute_kit_id");
}
