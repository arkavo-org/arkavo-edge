#!/bin/bash

echo "Testing State Management Tools"
echo "=============================="

# Test mutate_state - create
echo -e "\n1. Creating entity with mutate_state:"
echo '{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"mutate_state","arguments":{"entity":"user1","action":"create","data":{"name":"John","age":30}}}}' | \
cargo run --bin arkavo -- serve 2>/dev/null | tail -1 | jq -r '.result.content[0].text' | jq .

# Test query_state
echo -e "\n2. Querying created entity:"
echo '{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"query_state","arguments":{"entity":"user1"}}}' | \
cargo run --bin arkavo -- serve 2>/dev/null | tail -1 | jq -r '.result.content[0].text' | jq .

# Test snapshot - create
echo -e "\n3. Creating snapshot:"
echo '{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"snapshot","arguments":{"action":"create","name":"before_update"}}}' | \
cargo run --bin arkavo -- serve 2>/dev/null | tail -1 | jq -r '.result.content[0].text' | jq .

# Test mutate_state - update
echo -e "\n4. Updating entity:"
echo '{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"mutate_state","arguments":{"entity":"user1","action":"update","data":{"age":31,"city":"NYC"}}}}' | \
cargo run --bin arkavo -- serve 2>/dev/null | tail -1 | jq -r '.result.content[0].text' | jq .

# Test query all
echo -e "\n5. Query all entities:"
echo '{"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"query_state","arguments":{"entity":"*"}}}' | \
cargo run --bin arkavo -- serve 2>/dev/null | tail -1 | jq -r '.result.content[0].text' | jq .

# Test snapshot - list
echo -e "\n6. List snapshots:"
echo '{"jsonrpc":"2.0","id":6,"method":"tools/call","params":{"name":"snapshot","arguments":{"action":"list"}}}' | \
cargo run --bin arkavo -- serve 2>/dev/null | tail -1 | jq -r '.result.content[0].text' | jq .