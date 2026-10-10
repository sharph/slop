#!/usr/bin/env bash
# Integration tests for slop using the mock LLM server.
# Usage: ./test/run_tests.sh

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_DIR="$(dirname "$SCRIPT_DIR")"
SLOP="$PROJECT_DIR/target/debug/slop"
MOCK_PORT=8080
MOCK_URL="http://127.0.0.1:$MOCK_PORT/v1"

# Colors
GREEN='\033[0;32m'
RED='\033[0;31m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

PASS=0
FAIL=0

pass() {
    echo -e "${GREEN}PASS${NC}: $1"
    PASS=$((PASS + 1))
}

fail() {
    echo -e "${RED}FAIL${NC}: $1"
    if [ -n "${2:-}" ]; then
        echo "  Expected: $2"
        echo "  Got:      $3"
    fi
    FAIL=$((FAIL + 1))
}

assert_eq() {
    local name="$1"
    local expected="$2"
    local actual="$3"
    if [ "$expected" = "$actual" ]; then
        pass "$name"
    else
        fail "$name" "$expected" "$actual"
    fi
}

# Build
echo -e "${YELLOW}Building slop...${NC}"
cd "$PROJECT_DIR"
cargo build 2>&1 | tail -1

# Start mock server
echo -e "${YELLOW}Starting mock LLM server on port $MOCK_PORT...${NC}"
python3 "$SCRIPT_DIR/mock_llm.py" "$MOCK_PORT" 2>/dev/null &
MOCK_PID=$!
trap "kill $MOCK_PID 2>/dev/null || true" EXIT
sleep 1

SLOP_CMD="$SLOP --api-base $MOCK_URL --model mock"

echo ""
echo "=== Test 1: Simple text generation (no tools) ==="
RESULT=$($SLOP_CMD "generate numbers between 1 and 5, one number per line" 2>/dev/null)
assert_eq "simple generation" "$(printf '1\n2\n3\n4\n5')" "$RESULT"

echo ""
echo "=== Test 2: Fibonacci via run_command (stdin=TTY) ==="
RESULT=$($SLOP_CMD "generate fib numbers < 100" 2>/dev/null | head -5)
assert_eq "fibonacci first 5" "$(printf '0\n1\n1\n2\n3')" "$RESULT"

echo ""
echo "=== Test 3: Full pipeline chain ==="
RESULT=$($SLOP_CMD "generate numbers between 1 and 5, one number per line" 2>/dev/null \
    | $SLOP_CMD "sort list numerically" 2>/dev/null \
    | $SLOP_CMD "for each line, emit one x for the number specified" 2>/dev/null)
assert_eq "full chain" "$(printf 'x\nxx\nxxx\nxxxx\nxxxxx')" "$RESULT"

echo ""
echo "=== Test 4: Sort with piped input ==="
RESULT=$(printf '5\n3\n1\n4\n2\n' | $SLOP_CMD "sort list numerically" 2>/dev/null)
assert_eq "sort piped input" "$(printf '1\n2\n3\n4\n5')" "$RESULT"

echo ""
echo "=== Test 5: Filter 404s | Most popular pages ==="
LOG_DATA='192.168.1.1 - /home 200
192.168.1.2 - /about 404
192.168.1.3 - /home 200
192.168.1.4 - /products 404
192.168.1.5 - /home 200'
RESULT=$(printf '%s\n' "$LOG_DATA" \
    | $SLOP_CMD "filter out 404s" 2>/dev/null \
    | $SLOP_CMD "what are the most popular pages?" 2>/dev/null)
echo "  Output: $RESULT"
if echo "$RESULT" | grep -q "/home"; then
    pass "log analysis pipeline"
else
    fail "log analysis pipeline" "output containing /home" "$RESULT"
fi

echo ""
echo "=== Test 6: Large input streaming (10k lines) ==="
RESULT=$(seq 1 10000 | shuf | $SLOP_CMD "sort list numerically" 2>/dev/null | wc -l)
assert_eq "streaming 10k lines" "10000" "$RESULT"

echo ""
echo "=== Test 7: Exit code propagation ==="
printf 'test\n' | $SLOP_CMD "sort list numerically" 2>/dev/null
EXIT_CODE=$?
assert_eq "exit code 0 on success" "0" "$EXIT_CODE"

echo ""
echo "=== Test 8: Missing model error ==="
# Use a clean HOME so no config file is found
RESULT=$(echo "test" | HOME=/tmp/slop-test-no-config "$PROJECT_DIR/target/debug/slop" "hello" 2>&1 || true)
if echo "$RESULT" | grep -q "No model specified"; then
    pass "missing model error"
else
    fail "missing model error" "error about missing model" "$RESULT"
fi

echo ""
echo "=== Test 9: API connection error ==="
RESULT=$("$PROJECT_DIR/target/debug/slop" --api-base http://127.0.0.1:9999/v1 --model mock "test" 2>&1 || true)
if echo "$RESULT" | grep -q "Failed to send request"; then
    pass "API connection error"
else
    fail "API connection error" "error about connection" "$RESULT"
fi

echo ""
echo "=== Test 10: Env var config ==="
RESULT=$(SLOP_MODEL=mock SLOP_API_BASE="$MOCK_URL" "$SLOP" "generate numbers between 1 and 5, one number per line" 2>/dev/null)
assert_eq "env var config" "$(printf '1\n2\n3\n4\n5')" "$RESULT"

echo ""
echo "=== Test 11: --help ==="
RESULT=$("$SLOP" --help 2>&1)
if echo "$RESULT" | grep -q "pipe-aware LLM harness"; then
    pass "--help output"
else
    fail "--help output" "help text" "$RESULT"
fi

echo ""
echo "=== Test 12: --version ==="
RESULT=$("$SLOP" --version 2>&1)
if echo "$RESULT" | grep -q "slop 0.1.0"; then
    pass "--version output"
else
    fail "--version output" "version string" "$RESULT"
fi

# Summary
echo ""
echo "========================================"
echo -e "Results: ${GREEN}$PASS passed${NC}, ${RED}$FAIL failed${NC}"
echo "========================================"

if [ "$FAIL" -gt 0 ]; then
    exit 1
fi
