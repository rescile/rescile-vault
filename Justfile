# Justfile for the rescile-vault Rust project

# ==============================================================================
# VARIABLES
# ==============================================================================
name := "rescile-vault"

# Output directory for release binaries
out_dir := "target/release"
static_out_dir := "target/x86_64-unknown-linux-musl/release"

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

# Build the rescile-vault binary in static release mode for linux (musl)
build-static:
    #!/usr/bin/env bash
    set -euo pipefail

    # Revert Cargo.toml on exit to restore the original version.
    trap 'sed -i "s/version = \"${new_version}\"/version = \"${current_version}\"/" Cargo.toml' EXIT

    # Read current version from Cargo.toml and increment the patch number
    current_version=$(grep '^version = ' Cargo.toml | sed -E 's/version = "(.+)"/\1/')
    major_minor=$(echo "$current_version" | cut -d. -f1-2)
    patch=$(echo "$current_version" | cut -d. -f3)
    new_patch=$((patch + 1))
    new_version="${major_minor}.${new_patch}+$(date +%Y%m%d%H%M%S)"

    echo "==> Preparing developer build..."
    echo "    -> Temporarily setting version to ${new_version}"
    sed -i "s/version = \"${current_version}\"/version = \"${new_version}\"/" Cargo.toml

    # Set the release channel for the build, which the binary will embed
    export RESCILE_RELEASE_CHANNEL="developer-build"

    echo "    -> Building static '{{name}}' (developer build for x86_64-unknown-linux-musl)..."
    cargo build --release --target x86_64-unknown-linux-gnu --features openssl-vendored,tpm
    echo "==> Developer build complete. Version reverted in Cargo.toml."
    echo "    Binary is available at: {{static_out_dir}}/{{name}}"

# Run tests
test:
    @echo "Running tests..."
    @cargo test

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

# Create a new versioned release.
# This updates Cargo.toml, builds the Nix package, commits the changes, and creates a git tag.
# Usage: just release <VERSION>
# Example: just release 0.2.0
release new_version:
    #!/usr/bin/env bash
    set -euo pipefail

    # --- Variables ---
    NEW_VERSION="{{new_version}}"
    VERSION_TAG="v{{new_version}}"

    # --- Pre-flight Checks ---
    if [[ -z "${NEW_VERSION}" ]]; then
        echo "❌ ERROR: Version argument is mandatory." >&2
        echo "Usage: just release <VERSION>" >&2
        exit 1
    fi
    if ! git diff-index --quiet HEAD --; then
        echo "❌ ERROR: Working directory is not clean. Please commit or stash changes." >&2
        exit 1
    fi
    if git rev-parse "${VERSION_TAG}" >/dev/null 2>&1; then
        echo "❌ ERROR: Tag '${VERSION_TAG}' already exists." >&2
        exit 1
    fi

    echo "==> Releasing version '${VERSION_TAG}'..."

    # Step 1: Update Cargo.toml with the new version
    echo "    -> Updating Cargo.toml to version ${NEW_VERSION}..."
    sed -i -E "s/^version = \".*\"/version = \"${NEW_VERSION}\"/" Cargo.toml
    if [ -f package.nix ]; then
        echo "    -> Updating package.nix to version ${NEW_VERSION}..."
        sed -i -E "s/version = \".*\";/version = \"${NEW_VERSION}\";/" package.nix
    fi
    cargo update --package {{name}}

    # Step 2: Commit the changes
    echo "    -> Committing release files..."
    git add -u Cargo.toml Cargo.lock package.nix || git add -u Cargo.toml Cargo.lock
    git commit -m "Release ${VERSION_TAG}"

    echo
    echo "✅ Release commit for '${VERSION_TAG}' created successfully!"
    echo "    Next, push this branch and create a pull request."
    echo "    After the PR is merged, the release can be finalized by running:"
    echo "    just mark-release ${NEW_VERSION} <pre|release>"

nix-release new_version:
    #!/usr/bin/env bash
    set -euo pipefail

    # --- Variables ---
    NEW_VERSION="{{new_version}}"
    VERSION_TAG="v{{new_version}}"
    NIX_PKG_DIR="nix/pkgs/{{name}}"
    PACKAGE_NIX_FILE="${NIX_PKG_DIR}/package.nix"
    PACKAGE_TGZ="{{name}}-${NEW_VERSION}.tgz"

    # --- Pre-flight Checks ---
    if [[ -z "${NEW_VERSION}" ]]; then
        echo "❌ ERROR: Version argument is mandatory." >&2
        echo "Usage: just release <VERSION>" >&2
        exit 1
    fi
    
    if [ ! -f "${PACKAGE_NIX_FILE}" ]; then
        echo "ℹ️  No package.nix found at ${PACKAGE_NIX_FILE}. Skipping Nix release steps."
        exit 0
    fi

    echo "==> Nix Release version '${VERSION_TAG}'..."

    # Step 2: Create a source tarball that includes the updated Cargo.toml
    echo "    -> Creating source tarball ${PACKAGE_TGZ}..."
    PREFIX="{{name}}-${NEW_VERSION}"
    git ls-files | tar -czf "${PACKAGE_TGZ}" --transform="s,^,${PREFIX}/," -T -

     # Step 3: Update Nix package definition
    echo "    -> Updating Nix package definition..."
    sed -i -E "s/(version = \")[^\"]+/\1${NEW_VERSION}/" "${PACKAGE_NIX_FILE}"
    sed -i -E "s|({{name}}-)[^\"]+(\.tgz\")|\1${NEW_VERSION}\2|" "${PACKAGE_NIX_FILE}"
    cp "${PACKAGE_TGZ}" "${NIX_PKG_DIR}/"

    echo "    -> Calculating new src hash..."
    SRC_HASH=$(nix-prefetch-url --type sha256 "file://$(pwd)/${NIX_PKG_DIR}/${PACKAGE_TGZ}" | xargs nix hash convert --hash-algo sha256 --from nix32 --to sri)
    if [[ -z "$SRC_HASH" ]]; then
        echo "    -> ERROR: Could not calculate src hash for '${PACKAGE_TGZ}'." >&2; git checkout -- Cargo.toml; exit 1
    fi
    sed -i -E "s|hash = \\\".*\\\";|hash = \\\"${SRC_HASH}\\\";|" "${PACKAGE_NIX_FILE}"

    DUMMY_HASH="sha256-AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA="
    sed -i -E "s|cargoHash = \\\".*\\\";|cargoHash = \\\"${DUMMY_HASH}\\\";|" "${PACKAGE_NIX_FILE}"

    echo "    -> Calculating cargoHash (this will show a build error, which is expected)..."
    BUILD_OUTPUT=$(nix-build --no-out-link --expr "(import <nixpkgs> {}).callPackage ./${PACKAGE_NIX_FILE} {}" 2>&1 || true)
    CARGO_HASH=$(echo "${BUILD_OUTPUT}" | grep -E '(got:|observed:)' | grep -oE 'sha256-[-A-Za-z0-9+/=]+' | tail -n 1)

    if [[ -z "$CARGO_HASH" ]]; then
        echo "    -> ERROR: Could not determine cargoHash." >&2
        echo "    -> Full build output:" >&2; echo "${BUILD_OUTPUT}" >&2
        git checkout -- Cargo.toml "${PACKAGE_NIX_FILE}"; exit 1
    fi
    sed -i -E "s|cargoHash = \\\".*\\\";|cargoHash = \\\"${CARGO_HASH}\\\";|" "${PACKAGE_NIX_FILE}"
    echo "    -> Updated Nix package with src hash and cargoHash."

    # Step 4: Verify the Nix build is successful
    echo "    -> Verifying Nix build..."
    if ! nix-build --no-out-link --expr "(import <nixpkgs> {}).callPackage ./${PACKAGE_NIX_FILE} {}" >/dev/null; then
        echo "    -> ERROR: Nix build failed even after updating hashes." >&2
        git checkout -- Cargo.toml "${PACKAGE_NIX_FILE}"; exit 1
    fi
    echo "    -> Nix build successful!"

    # Step 5: Commit the changes
    echo "    -> Committing release files..."
    git add -u "${PACKAGE_NIX_FILE}"
    git add "${NIX_PKG_DIR}/${PACKAGE_TGZ}"
    git commit -m "Nix Release ${VERSION_TAG}"

    echo
    echo "✅ Nix release for '${VERSION_TAG}' created successfully!"
    echo "    Next, push this branch and create a pull request."


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
