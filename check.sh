#!/bin/bash

# Local checker, mirroring CI.
# Usage: ./check.sh [fmt] [clippy] [test] [itest]

set -o pipefail

if [ "$#" -eq 0 ]; then
    args=("fmt" "clippy" "test" "itest" "etest")
else
    args=("$@")
fi

for arg in "${args[@]}"; do
    if [ "$arg" == "--help" ]; then
        echo "Usage: check.sh [<commands>]"
        echo ""
        echo "Each specified command runs until one fails."
        echo "With no commands, all checks run."
        echo ""
        echo "Commands:"
        echo "    fmt           format code, fail if bad"
        echo "    clippy        validate clippy lints"
        echo "    test          run unit tests (no Godot)"
        echo "    itest         run integration tests (needs Godot 4)"
        echo "    etest         run the editor-mode integration test"
        echo "    doc           generate docs for the 'godot' crate"
        exit 0
    fi
done

# Godot 4 binary used by the integration tests.
function findGodot() {
    if [ -n "$GODOT4_BIN" ]; then
        godotBin="$GODOT4_BIN"
    elif [ -x "/Applications/Godot.app/Contents/MacOS/Godot" ]; then
        godotBin="/Applications/Godot.app/Contents/MacOS/Godot"
    elif command -v godot4 &>/dev/null; then
        godotBin="godot4"
    elif command -v godot &>/dev/null; then
        godotBin="godot"
    else
        echo "Godot 4 executable not found; set GODOT4_BIN"
        exit 2
    fi
    echo "Using Godot: $godotBin"
}

# The dynamic library extension differs per platform; the .gdextension lists all of them.
function libName() {
    case "$(uname -s)" in
        Darwin) echo "libitest.dylib" ;;
        Linux)  echo "libitest.so" ;;
        *)      echo "itest.dll" ;;
    esac
}

# A Godot project only picks up a .gdextension once its filesystem has been scanned, and the
# resulting .godot/ directory is not committed. A fresh clone (or CI) therefore has to scan
# once before any test can find the registered classes.
#
# The scan is done through the editor rather than --import: on 4.7.2 both crash after finishing
# on a project that registers a Node-derived exposed class (see README), so the exit code is
# ignored and the result is checked by looking for .godot/ instead.
function ensureImported() {
    local proj="$1"
    if [ -d "$proj/.godot" ]; then
        return 0
    fi

    echo "First run: scanning $proj (Godot 4.7.2 may crash after the scan; the scan still completes)"
    "$godotBin" --headless --path "$proj" --editor --quit >/dev/null 2>&1 || true

    if [ ! -d "$proj/.godot" ]; then
        echo "Godot did not produce $proj/.godot; the project could not be scanned"
        return 1
    fi
}

# Runs Godot as an editor, where an EditorPlugin asserts the editor half of the init-level
# gate. Godot exits 0 even when a plugin prints errors, so the success marker in the output is
# what decides, not the exit code.
function runEditorTest() {
    local output
    output=$("$godotBin" --headless --path itest/godot --editor --quit 2>&1)
    echo "$output" | grep -E "itest-editor"
    echo "$output" | grep -q "itest-editor: OK"
}

cmds=()

for arg in "${args[@]}"; do
    case "$arg" in
    fmt)
        cmds+=("cargo fmt --all -- --check")
        ;;
    clippy)
        cmds+=("cargo clippy --workspace -- -D clippy::style -D clippy::complexity -D clippy::perf -D clippy::dbg_macro -D clippy::todo -D clippy::unimplemented -D warnings")
        ;;
    test)
        cmds+=("cargo test --workspace")
        ;;
    itest)
        findGodot
        lib=$(libName)
        target_dir=$(cargo metadata --format-version 1 --no-deps | python3 -c 'import json,sys; print(json.load(sys.stdin)["target_directory"])')
        cmds+=("cargo build -p itest")
        cmds+=("cp $target_dir/debug/$lib itest/godot/lib/")
        cmds+=("ensureImported itest/godot")
        cmds+=("$godotBin --headless --path itest/godot")
        ;;
    etest)
        findGodot
        lib=$(libName)
        target_dir=$(cargo metadata --format-version 1 --no-deps | python3 -c 'import json,sys; print(json.load(sys.stdin)["target_directory"])')
        cmds+=("cargo build -p itest")
        cmds+=("cp $target_dir/debug/$lib itest/godot/lib/")
        cmds+=("ensureImported itest/godot")
        cmds+=("runEditorTest")
        ;;
    doc)
        cmds+=("cargo doc --lib -p godot --no-deps")
        ;;
    *)
        echo "Unrecognized command '$arg'"
        exit 2
        ;;
    esac
done

RED='\033[1;31m'
GREEN='\033[1;36m'
END='\033[0m'
for cmd in "${cmds[@]}"; do
    echo "> $cmd"
    $cmd || {
        printf "$RED\n=========================="
        printf "\ngodot-rs checker FAILED."
        printf "\n==========================\n$END"
        exit 1
    }
done

printf "$GREEN\n=============================="
printf "\ngodot-rs checker SUCCESSFUL."
printf "\n==============================\n$END"
