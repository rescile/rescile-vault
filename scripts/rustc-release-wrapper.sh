#!/usr/bin/env bash
# Wraps rustc for release builds so all host (proc-macro/build-script) and
# target binaries receive path remappings regardless of checkout directory.
set -euo pipefail
exec "$@" "--remap-path-prefix=$PWD=/src" "--remap-path-prefix=${CARGO_HOME:-$PWD/.cargo}=/cargo"
