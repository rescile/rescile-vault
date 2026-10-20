# Justfile for the rescile-vault Rust project

# ==============================================================================
# VARIABLES
# ==============================================================================
name := "rescile-vault"

# Output directory for release binaries
out_dir := "target/release"
static_out_dir := "target/nix-release-musl/x86_64-unknown-linux-musl/release"

# ==============================================================================
# DEVELOPMENT
# ==============================================================================

# The default recipe, executed when running `just` with no arguments.
# It provides a list of all available commands, acting as a help menu.
default:
    @just --list

# Build the rescile-vault binary in release mode
build:
    @echo "Building {{name}} (release mode)..."
    @cargo build --release
    @echo "Build complete. Binary created in {{out_dir}}/"

# Local and CI builds use the same pinned Nix release shell and checks.
build-release profile="release-musl":
    #!/usr/bin/env bash
    set -euo pipefail
    if [[ "{{profile}}" != release-musl ]]; then
        echo "Unknown release profile: {{profile}}" >&2; exit 1
    fi
    if [[ "${RESCILE_RELEASE_PROFILE:-}" != release-musl ]]; then
        exec nix develop --no-update-lock-file .#release-musl --command just build-release release-musl
    fi
    target="${RESCILE_RELEASE_TARGET:?release shell must set target}"
    rustup target add "$target"
    export RESCILE_RELEASE_CHANNEL="${RESCILE_RELEASE_CHANNEL:-developer-build}"
    export SOURCE_DATE_EPOCH="$(git log -1 --format=%ct HEAD)"
    export RUSTC_WRAPPER="$PWD/scripts/rustc-release-wrapper.sh"
    export NIX_LDFLAGS="$(echo "${NIX_LDFLAGS:-}" | sed "s|-rpath $PWD[^ ]*||g")"
    if [[ "$target" == x86_64-unknown-linux-musl ]]; then
        export CARGO_TARGET_X86_64_UNKNOWN_LINUX_MUSL_RUSTFLAGS="${CARGO_TARGET_X86_64_UNKNOWN_LINUX_MUSL_RUSTFLAGS:-} -C target-cpu=x86-64-v2"
    fi
    npm ci --prefix ui --no-audit --no-fund
    npm run build --prefix ui
    SKIP_FRONTEND_BUILD=1 cargo build --release --locked --target "$target" --features tpm
    binary="${CARGO_TARGET_DIR:-target}/$target/release/rescile-vault"
    strip "$binary"
    if readelf -l "$binary" | grep -q INTERP || readelf -d "$binary" | grep -q NEEDED; then
        echo "Not a static musl binary: $binary" >&2; exit 1
    fi
    "$binary" --help | grep -q -- '--tpm'
    "$binary" --version

package-release profile="release-musl":
    #!/usr/bin/env bash
    set -euo pipefail
    if [[ "{{profile}}" != release-musl ]]; then
        echo "Unknown release profile: {{profile}}" >&2; exit 1
    fi
    arch=$(uname -m)
    case "$arch" in
        x86_64|aarch64) ;;
        *) echo "Unsupported release architecture: $arch" >&2; exit 1 ;;
    esac
    binary="target/nix-release-musl/${arch}-unknown-linux-musl/release/rescile-vault"
    name="rescile-vault-${arch}-linux-musl"
    mkdir -p dist
    cp "$binary" "dist/$name"
    (cd dist && sha256sum "$name" > "$name.sha256")

verify-release-repro profile="release-musl":
    #!/usr/bin/env bash
    set -euo pipefail
    if [[ "{{profile}}" != release-musl ]]; then
        echo "Unknown release profile: {{profile}}" >&2; exit 1
    fi
    bash scripts/verify-release-repro.sh release-musl "target/nix-release-musl/$(uname -m)-unknown-linux-musl/release/rescile-vault"

build-static: (build-release "release-musl")

# Run frontend build, Rust formatting, lint and tests before committing.
rust-check:
    #!/usr/bin/env bash
    set -euo pipefail
    npm ci --prefix ui --no-audit --no-fund
    npm run build --prefix ui
    cargo fmt --all -- --check
    SKIP_FRONTEND_BUILD=1 cargo clippy --locked --all-targets -- -D warnings
    SKIP_FRONTEND_BUILD=1 cargo test --locked

# Check Rust formatting
fmt:
    @cargo fmt --all -- --check

# Lint Rust code, including tests
clippy:
    @cargo clippy --all-targets -- -D warnings

# Run tests
test:
    @echo "Running tests..."
    @cargo test

# Refresh Cargo/npm lockfiles within existing manifest ranges and advance nixos-unstable.
dependency-update:
    #!/usr/bin/env bash
    set -euo pipefail
    cargo update
    (cd ui && npm update --package-lock-only --ignore-scripts --no-audit --no-fund)
    nix flake update nixpkgs

# Upgrade Cargo/npm manifest requirements (including majors) and regenerate locks.
dependency-upgrade:
    #!/usr/bin/env bash
    set -euo pipefail
    cargo upgrade --incompatible allow
    (cd ui && npx --yes npm-check-updates@23 --upgrade --target latest)
    just dependency-update

# Clean build artifacts and packaged tarballs
clean:
    @echo "Cleaning artifacts..."
    @cargo clean
    @rm -f *.tgz


# ==============================================================================
# INSTALLATION
# ==============================================================================

# Install the static binary to your local system (default /usr/local/bin)
install dest="/usr/local/bin": build-static
    #!/usr/bin/env bash
    set -euo pipefail
    
    echo "==> Installing {{name}} to {{dest}}..."
    if [ ! -f "{{static_out_dir}}/{{name}}" ]; then
        echo "❌ ERROR: Static binary not found. Build might have failed."
        exit 1
    fi
    
    sudo cp "{{static_out_dir}}/{{name}}" "{{dest}}/{{name}}"
    sudo chmod 755 "{{dest}}/{{name}}"
    echo "✅ Successfully installed {{name}} to {{dest}}/{{name}}"


# ==============================================================================
# RELEASE MANAGEMENT
# ==============================================================================

# Prepare a version commit; tag and publish only after the release PR is merged.
release new_version:
    #!/usr/bin/env bash
    set -euo pipefail
    NEW_VERSION="{{new_version}}"
    VERSION_TAG="v${NEW_VERSION}"

    if [[ ! "$NEW_VERSION" =~ ^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.-]+)?$ ]]; then
        echo "Invalid version: $NEW_VERSION (expected X.Y.Z or X.Y.Z-suffix)" >&2
        exit 1
    fi
    if [[ -n "$(git status --porcelain)" ]]; then
        echo 'Working directory is not clean. Commit or stash changes first.' >&2
        exit 1
    fi
    if git rev-parse -q --verify "refs/tags/$VERSION_TAG" >/dev/null; then
        echo "Tag $VERSION_TAG already exists." >&2
        exit 1
    fi

    sed -i -E "s/^version = \"[^\"]+\"/version = \"${NEW_VERSION}\"/" Cargo.toml
    cargo update --package rescile-vault
    git add -- Cargo.toml Cargo.lock
    git commit -m "Release ${VERSION_TAG}"

    echo "Release commit created. After merge, run: just mark-release ${NEW_VERSION} <pre|release>"

# No standalone vault Nix package exists; Linux binaries are released through flake shells.
# Retain a clear error for existing callers instead of silently skipping work.
nix-release new_version:
    @echo "No standalone vault Nix package; use the flake-backed GitHub release workflow for v{{new_version}}" >&2
    @exit 1


# Tag the current HEAD for a release and push to upstream, triggering the GitHub release workflow.
# Usage: just mark-release <VERSION> <pre|release>
mark-release new_version release_type:
    #!/usr/bin/env bash
    set -euo pipefail

    NEW_VERSION="{{new_version}}"
    RELEASE_TYPE="{{release_type}}"
    VERSION_TAG="v${NEW_VERSION}"

    # --- Pre-flight Checks ---
    if ! command -v gh &> /dev/null; then echo "❌ ERROR: 'gh' (GitHub CLI) not found. Please install it to continue."; exit 1; fi
    if [[ -z "${NEW_VERSION}" || -z "${RELEASE_TYPE}" ]]; then
        echo "❌ ERROR: Version and release type arguments are mandatory." >&2; echo "Usage: just mark-release <VERSION> <pre|release>" >&2; exit 1
    fi
    if [[ "${RELEASE_TYPE}" != "pre" && "${RELEASE_TYPE}" != "release" ]]; then
        echo "❌ ERROR: Invalid release type '${RELEASE_TYPE}'. Must be 'pre' or 'release'." >&2; exit 1
    fi
    if ! git remote | grep -q '^upstream$'; then
        echo "❌ ERROR: 'upstream' remote not found. Please add the main repository as 'upstream'." >&2
        exit 1
    fi
    if git rev-parse "${VERSION_TAG}" >/dev/null 2>&1; then
        echo "❌ ERROR: Tag '${VERSION_TAG}' already exists." >&2; exit 1
    fi
    if ! gh auth status &>/dev/null; then
        echo "❌ ERROR: Not authenticated with GitHub CLI. Please run 'gh auth login'." >&2
        exit 1
    fi

    GH_FLAGS=""
    if [[ "${RELEASE_TYPE}" == "pre" ]]; then
        GH_FLAGS="--prerelease"
    fi

    echo "==> Creating and pushing tag ${VERSION_TAG} to upstream..."
    git tag -a "${VERSION_TAG}" -m "Release ${VERSION_TAG}"
    git push upstream "${VERSION_TAG}"
    echo "==> Creating GitHub release for ${VERSION_TAG}..."
    gh release create "${VERSION_TAG}" \
        --repo "rescile/rescile-vault" \
        --title "${VERSION_TAG}" \
        --generate-notes \
        ${GH_FLAGS}

    echo "✅ GitHub release for '${VERSION_TAG}' created. Workflow will now build and upload assets."

# Publish release assets to Cloudflare R2.
publish-release version:
    #!/usr/bin/env bash
    set -euo pipefail

    RELEASE_TAG="v{{version}}"

    if [[ -z "{{version}}" ]]; then echo "❌ ERROR: Version is missing. Usage: just publish-release <VERSION>"; exit 1; fi
    if ! command -v gh &> /dev/null; then echo "❌ 'gh' (GitHub CLI) not found."; exit 1; fi
    if ! command -v aws &> /dev/null; then echo "❌ 'aws' (AWS CLI) not found."; exit 1; fi
    if ! command -v jq &> /dev/null; then echo "❌ 'jq' not found."; exit 1; fi
    : "${R2_BUCKET_NAME?R2_BUCKET_NAME env var not set}"
    : "${CLOUDFLARE_ENDPOINT?CLOUDFLARE_ENDPOINT env var not set}"
    : "${RESCILE_UPDATES_BUCKET_URL?RESCILE_UPDATES_BUCKET_URL env var not set}"

    TEMP_DIR=$(mktemp -d); trap 'rm -rf -- "$TEMP_DIR"' EXIT
    echo "==> Downloading assets for tag '${RELEASE_TAG}' to ${TEMP_DIR}"
    gh release download --repo "https://github.com/rescile/rescile-vault" "${RELEASE_TAG}" --dir "${TEMP_DIR}" --pattern '*'
    cd "${TEMP_DIR}"

    echo "==> Checking release type from GitHub..."
    IS_PRERELEASE=$(gh release view "${RELEASE_TAG}" --repo "https://github.com/rescile/rescile-vault" --json isPrerelease --jq .isPrerelease)

    if [[ "${IS_PRERELEASE}" == "true" ]]; then
        RELEASE_TYPE_STR="PRE-RELEASE"
    elif [[ "${IS_PRERELEASE}" == "false" ]]; then
        RELEASE_TYPE_STR="STABLE release"
    else
        echo "❌ ERROR: Could not determine release type from GitHub for tag '${RELEASE_TAG}'." >&2; exit 1
    fi
    echo "    -> Detected as a ${RELEASE_TYPE_STR}."

    echo "==> Uploading assets to R2 bucket '${R2_BUCKET_NAME}' for ${RELEASE_TAG}..."
    aws s3 cp --endpoint-url "${CLOUDFLARE_ENDPOINT}" . "s3://${R2_BUCKET_NAME}/${RELEASE_TAG}/" --recursive --exclude "index.json*"

    echo "==> Generating and uploading update index..."
    aws s3 cp --endpoint-url "${CLOUDFLARE_ENDPOINT}" "s3://${R2_BUCKET_NAME}/index.json" index.json.old || echo "{}" > index.json.old

    BASE_URL="${RESCILE_UPDATES_BUCKET_URL}/${RELEASE_TAG}"
    ASSETS_JSON=$(awk -v base_url="$BASE_URL" '{
        gsub(/\r/,"");
        asset_name = $2;
        sub(/^rescile-vault-/, "", asset_name);
        sub(/\.exe$/, "", asset_name);
        printf "{ \"%s\": { \"url\": \"%s/%s\", \"sha256\": \"%s\" } }\n", asset_name, base_url, $2, $1;
    }' checksums.txt | jq -s 'add')

    if [[ "${IS_PRERELEASE}" == "false" ]]; then
        echo "    -> Updating index for a STABLE release."
        jq --argjson assets "$ASSETS_JSON" --arg tag "$RELEASE_TAG" '.[$tag] = $assets | .["release"] = $assets' index.json.old > index.json
    else
        echo "    -> Updating index for a PRE-RELEASE."
        jq --argjson assets "$ASSETS_JSON" --arg tag "$RELEASE_TAG" '.[$tag] = $assets | .["pre"] = $assets' index.json.old > index.json
    fi
    aws s3 cp --endpoint-url "${CLOUDFLARE_ENDPOINT}" index.json "s3://${R2_BUCKET_NAME}/index.json" --acl public-read

    echo "✅ Release '${RELEASE_TAG}' published to R2."

# Remove an old release from R2.
remove-release version:
    #!/usr/bin/env bash
    set -euo pipefail

    RELEASE_TAG="v{{version}}"

    if [[ -z "{{version}}" ]]; then echo "❌ ERROR: Version is missing. Usage: just remove-release <VERSION>"; exit 1; fi
    if ! command -v aws &> /dev/null; then echo "❌ 'aws' (AWS CLI) not found."; exit 1; fi
    if ! command -v jq &> /dev/null; then echo "❌ 'jq' not found."; exit 1; fi
    : "${R2_BUCKET_NAME?R2_BUCKET_NAME env var not set}"
    : "${CLOUDFLARE_ENDPOINT?CLOUDFLARE_ENDPOINT env var not set}"

    echo "==> Removing release '${RELEASE_TAG}' from R2 bucket '${R2_BUCKET_NAME}'..."

    echo "    -> Deleting asset directory..."
    aws s3 rm --endpoint-url "${CLOUDFLARE_ENDPOINT}" "s3://${R2_BUCKET_NAME}/${RELEASE_TAG}/" --recursive

    echo "    -> Updating index.json..."
    TEMP_DIR=$(mktemp -d); trap 'rm -rf -- "$TEMP_DIR"' EXIT
    cd "${TEMP_DIR}"
    aws s3 cp --endpoint-url "${CLOUDFLARE_ENDPOINT}" "s3://${R2_BUCKET_NAME}/index.json" index.json.old

    # Remove the version key. If it was the latest stable release, remove the 'release' key as well.
    jq --arg tag "$RELEASE_TAG" 'if .release == .[$tag] then del(.[$tag], .release) else del(.[$tag]) end' index.json.old > index.json
    aws s3 cp --endpoint-url "${CLOUDFLARE_ENDPOINT}" index.json "s3://${R2_BUCKET_NAME}/index.json" --acl public-read

    echo "✅ Release '${RELEASE_TAG}' removed successfully."

# Show all released and pre-released versions published on R2.
show-releases:
    #!/usr/bin/env bash
    set -euo pipefail

    if ! command -v curl &> /dev/null; then echo "❌ 'curl' not found."; exit 1; fi
    if ! command -v jq &> /dev/null; then echo "❌ 'jq' not found."; exit 1; fi
    : "${RESCILE_UPDATES_BUCKET_URL?RESCILE_UPDATES_BUCKET_URL env var not set}"

    INDEX_URL="${RESCILE_UPDATES_BUCKET_URL}/index.json"
    echo "==> Fetching release index from ${INDEX_URL}..."
    echo

    curl -sL "${INDEX_URL}" | jq -r '
        . as $root |
        # Find the tag name corresponding to the latest stable release assets
        ( (keys_unsorted | map(select(. != "release" and . != "pre" and ($root[.] | type) == "object")) | .[] | select($root[.] == $root.release)) // null ) as $latest_tag |

        "Published Releases on R2:",
        "---------------------------",
        (keys_unsorted | map(select(. != "release" and ($root[.] | type) == "object")) | sort | .[] |
        if . == $latest_tag then
            "\(.) (latest stable)"
        else
            .
        end)
    '

# Show all released and pre-released versions published on GitHub.
show-gh-releases:
    #!/usr/bin/env bash
    set -euo pipefail

    if ! command -v gh &> /dev/null; then echo "❌ 'gh' (GitHub CLI) not found."; exit 1; fi

    echo "==> GitHub Releases and Pre-Releases:"
    echo "-------------------------------------------"
    gh release list --repo rescile/rescile-vault --limit 20

# Show recent Action workflows on GitHub.
show-gh-actions:
    #!/usr/bin/env bash
    set -euo pipefail

    if ! command -v gh &> /dev/null; then echo "❌ 'gh' (GitHub CLI) not found."; exit 1; fi

    echo "==> Recent GitHub Action Workflows:"
    echo "-------------------------------------------"
    gh run list --repo rescile/rescile-vault --limit 15

# Remove a release or pre-release from GitHub (keeps the git tag).
remove-gh-release version:
    #!/usr/bin/env bash
    set -euo pipefail

    RELEASE_TAG="v{{version}}"

    if [[ -z "{{version}}" ]]; then echo "❌ ERROR: Version is missing. Usage: just remove-gh-release <VERSION>"; exit 1; fi
    if ! command -v gh &> /dev/null; then echo "❌ 'gh' (GitHub CLI) not found."; exit 1; fi

    echo "==> Removing GitHub release '${RELEASE_TAG}'..."
    gh release delete "${RELEASE_TAG}" --repo "rescile/rescile-vault" --yes

    echo "✅ GitHub release '${RELEASE_TAG}' removed successfully."

# Remove GitHub Action workflow runs for a specific branch.
remove-gh-actions branch:
    #!/usr/bin/env bash
    set -euo pipefail

    BRANCH="{{branch}}"

    if [[ -z "${BRANCH}" ]]; then echo "❌ ERROR: Branch is missing. Usage: just remove-gh-actions <BRANCH>"; exit 1; fi
    if ! command -v gh &> /dev/null; then echo "❌ 'gh' (GitHub CLI) not found."; exit 1; fi

    echo "==> Removing GitHub Action runs for branch '${BRANCH}'..."
    RUN_IDS=$(gh run list --repo "rescile/rescile-vault" --branch "${BRANCH}" --json databaseId --jq '.[].databaseId')

    if [[ -z "${RUN_IDS}" ]]; then
        echo "    -> No runs found for branch '${BRANCH}'."
        exit 0
    fi

    for ID in ${RUN_IDS}; do
        echo "    -> Deleting run ${ID}..."
        gh run delete "${ID}" --repo "rescile/rescile-vault"
    done

    echo "✅ GitHub Action runs for branch '${BRANCH}' removed successfully."
