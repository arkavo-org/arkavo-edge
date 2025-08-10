#!/usr/bin/env python3
"""
Demo script showing multi-agent interaction via JSON-RPC

This demonstrates:
1. Querying the orchestrator agent
2. The orchestrator querying specialized agents
3. Agents sharing knowledge to solve a complex task
"""

import json
import asyncio
import aiohttp

async def call_agent_rpc(agent_url, method, params):
    """Make a JSON-RPC call to an agent"""
    payload = {
        "jsonrpc": "2.0",
        "method": method,
        "params": params,
        "id": 1
    }
    
    async with aiohttp.ClientSession() as session:
        async with session.post(agent_url, json=payload) as response:
            result = await response.json()
            return result

async def main():
    print("🤖 Arkavo Multi-Agent Collaboration Demo")
    print("=" * 50)
    
    # Agent endpoints
    orchestrator_url = "http://localhost:8342/rpc"
    
    # Scenario: User asks orchestrator to review code for security
    print("\n📝 User Query: 'Review my Python web app for security issues'")
    
    # Step 1: Send query to orchestrator
    print("\n1️⃣ Sending query to orchestrator agent...")
    
    # The orchestrator would use agent_query to ask other agents
    security_query = {
        "from_agent_id": "orchestrator-agent",
        "to_agent_id": "security-agent",
        "query": "Analyze Python web app for security vulnerabilities",
        "domain": "security"
    }
    
    print(f"   Orchestrator → Security Agent: {security_query['query']}")
    
    # Simulate security agent response
    print("\n2️⃣ Security agent analyzes code...")
    security_response = {
        "from_agent_id": "security-agent",
        "response": "Found SQL injection vulnerability in auth.py line 42",
        "confidence": 0.85,
        "evidence": {
            "file": "auth.py",
            "line": 42,
            "code": "query = f\"SELECT * FROM users WHERE username='{username}'\""
        }
    }
    print(f"   Security Agent: {security_response['response']}")
    print(f"   Confidence: {security_response['confidence'] * 100}%")
    
    # Step 3: Security agent queries code review agent
    print("\n3️⃣ Security agent asks code review agent for verification...")
    code_review_query = {
        "from_agent_id": "security-agent",
        "to_agent_id": "code-review-agent",
        "query": "Is this SQL query parameterized: SELECT * FROM users WHERE username='{username}'",
        "domain": "code_review"
    }
    print(f"   Security → Code Review: {code_review_query['query']}")
    
    # Code review agent response
    code_review_response = {
        "from_agent_id": "code-review-agent",
        "response": "No, uses string concatenation. Should use parameterized queries.",
        "confidence": 0.95
    }
    print(f"   Code Review Agent: {code_review_response['response']}")
    
    # Step 4: Orchestrator aggregates responses
    print("\n4️⃣ Orchestrator aggregates findings...")
    print("\n📊 Final Security Report:")
    print("-" * 40)
    print("CRITICAL: SQL Injection Vulnerability")
    print(f"- Location: {security_response['evidence']['file']}, line {security_response['evidence']['line']}")
    print("- Issue: Using string concatenation in SQL query")
    print("- Recommendation: Use parameterized queries")
    print(f"- Verified by 2 agents with {int((security_response['confidence'] + code_review_response['confidence']) / 2 * 100)}% confidence")
    
    print("\n✅ Demo complete!")
    print("\nThis demonstrates how agents:")
    print("- Share specialized knowledge")
    print("- Query each other for verification")
    print("- Collaborate to solve complex tasks")
    print("- Maintain context efficiency by asking specific questions")

if __name__ == "__main__":
    asyncio.run(main())