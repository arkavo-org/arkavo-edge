#!/usr/bin/env python3
"""GridColony policy probe — measures spatial decision quality across scenarios.

Drives two scripted policies through the Game-RL reference environment over
real MCP stdio (the same wire path an Edge agent uses):

  center-biased   : EstablishFarm anchored to MapCenter (the historical failure)
  landmark-aware  : observe Landmarks, anchor to the largest FertileCluster

The fertile-corner scenario is the regression probe: a center-biased policy
must score 0 on farm_fertility_quality there; a landmark-aware policy ~1.3.

Usage:
  python3 playtest_probe.py [--steps 20] [--server path/to/game-rl-reference]
"""

import argparse
import json
import os
import subprocess
import sys
from pathlib import Path

SCENARIOS = ["default", "fertile-corner", "threat-south", "scattered-resources"]
SEED = 42


class McpClient:
    def __init__(self, server_path):
        self.proc = subprocess.Popen(
            [server_path],
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.DEVNULL,
            text=True,
        )
        self.next_id = 1

    def request(self, method, params):
        rid = self.next_id
        self.next_id += 1
        msg = json.dumps(
            {"jsonrpc": "2.0", "id": rid, "method": method, "params": params}
        )
        self.proc.stdin.write(msg + "\n")
        self.proc.stdin.flush()
        while True:
            line = self.proc.stdout.readline()
            if not line:
                raise RuntimeError(f"server closed during {method}")
            try:
                response = json.loads(line)
            except json.JSONDecodeError:
                continue
            if response.get("id") == rid:
                return response

    def initialize(self):
        result = self.request(
            "initialize",
            {
                "protocolVersion": "2025-11-25",
                "capabilities": {"tools": {}},
                "clientInfo": {"name": "playtest-probe", "version": "1.0"},
            },
        )
        self.proc.stdin.write(
            json.dumps({"jsonrpc": "2.0", "method": "notifications/initialized"}) + "\n"
        )
        self.proc.stdin.flush()
        return result

    def tool(self, name, arguments):
        """Returns (ok, payload_or_error_message)."""
        response = self.request(
            "tools/call", {"name": name, "arguments": arguments}
        )
        if "error" in response:
            return False, response["error"].get("message", "")
        result = response.get("result", {})
        text = (result.get("content") or [{}])[0].get("text", "")
        if result.get("isError"):
            return False, text
        try:
            return True, json.loads(text)
        except json.JSONDecodeError:
            return True, text

    def close(self):
        try:
            self.proc.stdin.close()
            self.proc.wait(timeout=3)
        except Exception:
            self.proc.kill()


def run_policy(server_path, scenario, policy, steps):
    """Returns episode metrics for one policy on one scenario."""
    client = McpClient(server_path)
    try:
        client.initialize()
        client.tool("registerAgent", {"AgentId": "probe", "AgentType": "Controller"})
        client.tool("reset", {"Seed": SEED, "Scenario": scenario})

        def step(action, ticks=0):
            return client.tool(
                "step", {"AgentId": "probe", "Action": action, "Ticks": ticks}
            )

        # Keep the colony fed identically under both policies
        step({"Type": "Harvest", "Target": "Berries"})
        step({"Type": "Harvest", "Target": "Berries"})

        # The policy decision under test: where does the farm go?
        if policy == "center-biased":
            farm_ok, farm_detail = step(
                {"Type": "EstablishFarm", "Near": "MapCenter", "Size": 16}
            )
        else:  # landmark-aware
            ok, obs = client.tool("observe", {"Include": ["landmarks"], "Limit": 50})
            landmarks = []
            if isinstance(obs, dict):
                # Landmarks may sit at top level or nested under Observation
                landmarks = (
                    obs.get("Landmarks")
                    or obs.get("Observation", {}).get("Landmarks")
                    or []
                )
            clusters = [
                l["Id"]
                for l in landmarks
                if isinstance(l, dict) and str(l.get("Id", "")).startswith("FertileCluster")
            ]
            anchor = clusters[0] if clusters else "ColonyCenter"
            farm_ok, farm_detail = step(
                {"Type": "EstablishFarm", "Near": anchor, "Size": 16}
            )

        # Let the world run; defend if the scenario brings hostiles
        for _ in range(steps):
            ok, payload = step({"Type": "Wait"}, ticks=120)
            alerts = (payload.get("Alerts") if isinstance(payload, dict) else None) or []
            if any(a.get("Label") == "UnderAttack" for a in alerts if isinstance(a, dict)):
                step({"Type": "DefendColony"})

        ok, summary = client.tool("episodeSummary", {})
        breakdown = summary.get("RewardBreakdown", {}) if isinstance(summary, dict) else {}
        return {
            "scenario": scenario,
            "policy": policy,
            "farm_placed": bool(farm_ok),
            "farm_error": None if farm_ok else str(farm_detail)[:120],
            "farm_fertility_quality": round(breakdown.get("farm_fertility_quality", 0.0), 3),
            "food_production": round(breakdown.get("food_production", 0.0), 3),
            "total_reward": round(summary.get("TotalReward", 0.0), 3)
            if isinstance(summary, dict)
            else 0.0,
        }
    finally:
        client.close()


def default_server():
    candidates = [
        os.environ.get("GAME_RL_REFERENCE"),
        str(Path.home() / "Projects/intelligence/game-rl/target/release/game-rl-reference"),
        str(Path.home() / "Projects/intelligence/game-rl/target/debug/game-rl-reference"),
    ]
    for c in candidates:
        if c and Path(c).is_file():
            return c
    return "game-rl-reference"  # hope for PATH


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--server", default=default_server())
    parser.add_argument("--steps", type=int, default=20)
    parser.add_argument("--json", help="write results JSON to this path")
    args = parser.parse_args()

    results = []
    for scenario in SCENARIOS:
        for policy in ["center-biased", "landmark-aware"]:
            result = run_policy(args.server, scenario, policy, args.steps)
            results.append(result)

    header = f"{'scenario':<21} {'policy':<15} {'farm':<5} {'fertility':>9} {'food':>7} {'total':>8}"
    print(header)
    print("-" * len(header))
    for r in results:
        print(
            f"{r['scenario']:<21} {r['policy']:<15} "
            f"{'yes' if r['farm_placed'] else 'NO':<5} "
            f"{r['farm_fertility_quality']:>9} {r['food_production']:>7} {r['total_reward']:>8}"
        )

    # The probe assertion: on fertile-corner, landmark-aware must beat center-biased
    probe = {r["policy"]: r for r in results if r["scenario"] == "fertile-corner"}
    passed = (
        probe["landmark-aware"]["farm_fertility_quality"] >= 1.0
        and probe["center-biased"]["farm_fertility_quality"] == 0.0
    )
    print(
        f"\nfertile-corner probe: {'PASS' if passed else 'FAIL'} — "
        f"landmark-aware fertility {probe['landmark-aware']['farm_fertility_quality']} "
        f"vs center-biased {probe['center-biased']['farm_fertility_quality']}"
    )

    if args.json:
        with open(args.json, "w") as f:
            json.dump({"seed": SEED, "steps": args.steps, "results": results}, f, indent=2)
        print(f"results written to {args.json}")

    sys.exit(0 if passed else 1)


if __name__ == "__main__":
    main()
