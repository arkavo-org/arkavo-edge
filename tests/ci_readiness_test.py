#!/usr/bin/env python3
"""Protect the aggregate check that main requires before a merge."""

import os
from pathlib import Path
import re
import subprocess
import textwrap
import unittest


class ReadinessGateTest(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        workflow = (
            Path(__file__).resolve().parents[1] / ".github/workflows/feature.yaml"
        ).read_text()
        gate = workflow.split("\n  release-readiness:\n", 1)[1]
        condition = re.search(r"^    if: (.+)$", gate, re.MULTILINE).group(1).strip()
        cls.condition = condition.removeprefix("${{").removesuffix("}}").strip()
        needs = re.search(r"^    needs: \[([^\]]+)\]$", gate, re.MULTILINE)
        cls.dependencies = [item.strip() for item in needs.group(1).split(",")]
        body = re.search(
            r"^        run: \|\n((?:          .*\n|\n)+)", gate, re.MULTILINE
        ).group(1)
        cls.script = textwrap.dedent(body)

    def test_gate_is_never_skipped_after_failure_or_cancellation(self):
        # A skipped check satisfies branch protection. This gate is the sole
        # required context, so every upstream outcome must reach its script.
        self.assertEqual(self.condition, "always()")

    def test_every_required_dependency_is_checked(self):
        references = re.findall(r"needs\.([\w-]+)\.result", self.script)
        self.assertEqual(set(references), set(self.dependencies))

    def run_gate(self, outcomes):
        script = self.script
        for dependency, outcome in outcomes.items():
            script = script.replace(
                "${{ needs." + dependency + ".result }}", outcome
            )
        self.assertNotIn("${{", script)
        return subprocess.run(
            ["bash", "--noprofile", "--norc", "-e", "-c", script],
            env={"PATH": os.defpath},
            capture_output=True,
            text=True,
            check=False,
        )

    def test_complete_success_is_accepted(self):
        result = self.run_gate(dict.fromkeys(self.dependencies, "success"))
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)

    def test_incomplete_or_failed_required_checks_are_rejected(self):
        for dependency in self.dependencies:
            for outcome in ("failure", "cancelled", "skipped", "", "unknown"):
                with self.subTest(dependency=dependency, outcome=outcome):
                    outcomes = dict.fromkeys(self.dependencies, "success")
                    outcomes[dependency] = outcome
                    result = self.run_gate(outcomes)
                    self.assertNotEqual(result.returncode, 0)


if __name__ == "__main__":
    unittest.main()
