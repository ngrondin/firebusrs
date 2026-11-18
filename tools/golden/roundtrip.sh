#!/bin/bash
# Two-way golden-vector round-trip test between the Java and Rust firebus core.
#
#   FORWARD  (Java  -> Rust): the original Java implementation serialises fixed values
#        (VectorGen.java) into tests/golden/java_*.hex; the Rust tests parse them and
#        assert the logical content (cargo test).
#
#   REVERSE  (Rust  -> Java): the Rust implementation serialises the same values
#        (cargo run --bin gen_golden -> tests/golden/rust_*.hex); VectorVerify.java
#        parses them with the Java Message/Payload classes and asserts the fields.
#
# Regenerating fixtures:
#   Java -> Rust fixtures : tools/golden/gen.sh     (requires JDK + firebus sources)
#   Rust -> Java fixtures : cargo run --bin gen_golden
# Re-check the Rust bytes against the Java parser: tools/golden/verify.sh (requires JDK)

set -e
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"

echo "=== Direction 1: Java golden vectors read by Rust (cargo test) ==="
(cd "$ROOT" && cargo test)

echo ""
echo "=== Direction 2: Rust golden vectors read by Java (VectorVerify) ==="
bash "$ROOT/tools/golden/verify.sh"

echo ""
echo "TWO-WAY GOLDEN ROUND TRIP: PASS"
