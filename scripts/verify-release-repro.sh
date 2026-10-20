#!/usr/bin/env bash
# Compare release binary from a clean checkout with one built at another path.
set -euo pipefail

profile=${1:?release profile required}
binary=${2:?binary path required}
if [[ -n "$(git status --porcelain --untracked-files=normal)" ]]; then
    echo 'Reproducibility check requires a clean checkout.' >&2
    exit 1
fi
git ls-files --error-unmatch flake.nix flake.lock >/dev/null
export RESCILE_RELEASE_CHANNEL="${RESCILE_RELEASE_CHANNEL:-developer-build}"
export SOURCE_DATE_EPOCH="$(git log -1 --format=%ct HEAD)"

env -u RESCILE_RELEASE_PROFILE just build-release "$profile"
mkdir -p target
checkout=$(mktemp -d -p target repro.XXXXXXXX)
git clone --quiet --local --no-hardlinks . "$checkout/source"
git -C "$checkout/source" checkout --quiet --detach "$(git rev-parse HEAD)"
(cd "$checkout/source" && env -u RESCILE_RELEASE_PROFILE just build-release "$profile")

if cmp -s "$binary" "$checkout/source/$binary"; then
    sha256sum "$binary"
    echo 'Release binaries match.'
else
    sha256sum "$binary" "$checkout/source/$binary"
    echo "Release binaries differ. Compare with: diffoscope '$binary' '$checkout/source/$binary'" >&2
    exit 1
fi
