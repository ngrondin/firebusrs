#!/bin/bash
# Regenerates the Java golden vectors used by the Rust wire-format tests.
set -e
JAVA_SRC="${JAVA_SRC:-/home/nicolas/Source/firebus}"
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
OUT="$ROOT/target/golden-classes"
FIX="$ROOT/tests/golden"
mkdir -p "$OUT" "$FIX"
javac -d "$OUT" -sourcepath "$JAVA_SRC/FireBus/src/main/java:$JAVA_SRC/FirebusData/src/main/java" "$ROOT/tools/golden/VectorGen.java"
java -cp "$OUT" VectorGen "$FIX"
