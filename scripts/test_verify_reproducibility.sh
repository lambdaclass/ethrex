#!/bin/bash
# Tests for verify-reproducibility.sh
#
# Run with:
#   ./scripts/test_verify_reproducibility.sh
#
# Note: These tests validate argument parsing and error handling only.
# Full Docker-based testing requires the ere-compiler images.

# Don't use set -e as we're testing for failures
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SCRIPT="$SCRIPT_DIR/verify-reproducibility.sh"

PASS=0
FAIL=0

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
NC='\033[0m' # No Color

pass() {
    echo -e "${GREEN}PASS${NC}: $1"
    PASS=$((PASS + 1))
}

fail() {
    echo -e "${RED}FAIL${NC}: $1"
    FAIL=$((FAIL + 1))
}

echo "=============================================="
echo "Tests for verify-reproducibility.sh"
echo "=============================================="
echo ""

# Test 1: Invalid zkVM argument
echo "Test 1: Invalid zkVM argument should fail"
OUTPUT=$("$SCRIPT" invalid_zkvm 2>&1) || true
if echo "$OUTPUT" | grep -q "Invalid zkVM"; then
    pass "Invalid zkVM detected correctly"
else
    fail "Expected 'Invalid zkVM' error message, got: $OUTPUT"
fi

# Test 2: Valid zkVM arguments (sp1, risc0, zisk)
# These will fail later due to Docker not being available or no guest program,
# but should pass the validation phase
echo ""
echo "Test 2: Valid zkVM 'sp1' should pass validation"
OUTPUT=$("$SCRIPT" sp1 2>&1) || true
if echo "$OUTPUT" | grep -q "zkVM: sp1"; then
    pass "sp1 zkVM accepted"
else
    fail "sp1 zkVM not recognized"
fi

echo ""
echo "Test 3: Valid zkVM 'risc0' should pass validation"
OUTPUT=$("$SCRIPT" risc0 2>&1) || true
if echo "$OUTPUT" | grep -q "zkVM: risc0"; then
    pass "risc0 zkVM accepted"
else
    fail "risc0 zkVM not recognized"
fi

echo ""
echo "Test 4: Valid zkVM 'zisk' should pass validation (default)"
OUTPUT=$("$SCRIPT" zisk 2>&1) || true
if echo "$OUTPUT" | grep -q "zkVM: zisk"; then
    pass "zisk zkVM accepted"
else
    fail "zisk zkVM not recognized"
fi

# Test 5: Default zkVM is zisk
echo ""
echo "Test 5: Default zkVM should be 'zisk'"
OUTPUT=$("$SCRIPT" 2>&1) || true
if echo "$OUTPUT" | grep -q "zkVM: zisk"; then
    pass "Default zkVM is zisk"
else
    fail "Default zkVM is not zisk"
fi

# Test 6: Custom ere_version is displayed
echo ""
echo "Test 6: Custom ere_version should be displayed"
OUTPUT=$("$SCRIPT" sp1 custom-version-123 2>&1) || true
if echo "$OUTPUT" | grep -q "ere-compiler version: custom-version-123"; then
    pass "Custom ere_version displayed"
else
    fail "Custom ere_version not displayed"
fi

# Test 7: Default ere_version is 'latest'
echo ""
echo "Test 7: Default ere_version should be 'latest'"
OUTPUT=$("$SCRIPT" sp1 2>&1) || true
if echo "$OUTPUT" | grep -q "ere-compiler version: latest"; then
    pass "Default ere_version is latest"
else
    fail "Default ere_version is not latest"
fi

# Test 8: Script creates and cleans up temp directories
echo ""
echo "Test 8: Script should attempt cleanup on exit (trap test)"
# We verify the trap is set by checking the script source
if grep -q 'trap.*rm -rf' "$SCRIPT"; then
    pass "Cleanup trap is defined"
else
    fail "Cleanup trap not found"
fi

# Test 9: Script uses proper variable quoting
echo ""
echo "Test 9: Script should use proper variable quoting"
# Check for properly quoted sha256sum calls
if grep 'sha256sum "\${BUILD' "$SCRIPT" > /dev/null 2>&1; then
    pass "Variables appear properly quoted"
else
    fail "sha256sum variables not properly quoted"
fi

echo ""
echo "=============================================="
echo "Results: $PASS passed, $FAIL failed"
echo "=============================================="

if [ "$FAIL" -gt 0 ]; then
    exit 1
fi
exit 0
