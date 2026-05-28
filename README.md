# rescile-vault

End-to-end encrypted secret manager. A single static binary that talks to a
Rescile Vault server, exposes a CLI for scripts and a local web UI for humans,
and never lets the server see your secrets in cleartext.

All cryptographic operations happen on the client. The server only ever sees
opaque ciphertexts and public keys.

## Features

- **Zero-knowledge architecture** — Argon2id-derived keys, AES‑256‑GCM for
  data, X25519 for per-collection key wrapping. The server cannot read
  secrets, secret names, or collection membership material in cleartext.
- **Multi-client collections** — invite other clients to a collection,
  rotate/revoke access, and assign roles (Owner / Member / Reader).
- **Two unlock methods** — password (Argon2id) or TPM 2.0 (with the `tpm`
  build feature).
- **Embedded web UI** — `rescile-vault ui` starts a local server on a random
  port and serves a React UI bundled into the binary at build time.
- **Batch mode** — drive bulk secret creation / invites from a file or via the
  HTTP API.
- **Single static binary** — Linux musl build has no runtime dependencies.

## Install

### Download a release

Pre-built static Linux binaries are attached to each GitHub release. Pick the
asset for your platform:

| Asset | Target | Notes |
|---|---|---|
| `rescile-vault-vX.Y.Z-x86_64-linux-musl` | `x86_64-unknown-linux-musl` | Fully static. Recommended. |
| `rescile-vault-vX.Y.Z-x86_64-linux-gnu-tpm` | `x86_64-unknown-linux-gnu` + `tpm` | Includes TPM 2.0 support. tpm2-tss is statically linked from source; only glibc remains a dynamic dep. |

```sh
# Replace VERSION with the release you want
VERSION=v0.1.0
curl -L -o rescile-vault \
  "https://github.com/rescile/rescile-vault/releases/download/${VERSION}/rescile-vault-${VERSION}-x86_64-linux-musl"
chmod +x rescile-vault
sudo mv rescile-vault /usr/local/bin/
```

Each release also ships a `checksums.txt` with SHA‑256 sums:

```sh
sha256sum -c --ignore-missing checksums.txt
```

### Build from source

Requirements: a recent stable Rust toolchain and Node.js (the build script
runs `npm install && npm run build` in `ui/` and embeds the result).

```sh
cargo build --release
```

For a fully static Linux binary (matches the CI release artifacts), use the
Justfile recipe — it uses Zig as the C linker and enables the
`openssl-vendored` feature:

```sh
just build-static
# Output: target/x86_64-unknown-linux-musl/release/rescile-vault
```

Optional features:

| Feature | Effect |
|---|---|
| `openssl-vendored` | Build a vendored OpenSSL — required for the static musl build. |
| `tpm` | Enable TPM 2.0 unlock (`--tpm` flag). Pulls in `tss-esapi`. |

## Usage

```text
rescile-vault [OPTIONS] <COMMAND>

Options:
  --url <URL>                Vault base URL          [env: RESCILE_VAULT_URL] [default: http://localhost:7600]
  --clientname <NAME>        Client identity         [env: RESCILE_VAULT_CLIENTNAME] [default: $USER@$HOSTNAME]
  --password <PASSWORD>      Master password         [env: RESCILE_VAULT_PASSWORD]
  --tpm                      Use TPM instead of a password (requires `tpm` feature)

Commands:
  secret      get | put | delete
  collection  create | invite | delete | remove-client | role | revoke
  client      delete
  batch <FILE>
  ui
```

If no `--password` is supplied (and `--tpm` is not used), one is generated and
stored at `$XDG_CONFIG_HOME/rescile/vault.password` (or `/etc/rescile/vault.password`
for root) with `0600` permissions.

### Examples

```sh
# Read or auto-generate a secret
rescile-vault secret get default db_password

# Store a known value
rescile-vault secret put default api_token "sk-..."

# Create a collection and invite another client
rescile-vault collection create infra
rescile-vault collection invite infra --client alice@laptop --validity 1d
# → prints a one-time invite token; share over a secure channel

# Start the local web UI
rescile-vault ui
# → Web UI started at http://127.0.0.1:54321
```

### Batch mode

```sh
# secrets.batch
collection create infra
secret put infra db_password
secret put infra api_token "sk-..."
collection invite infra --client alice@laptop --validity 7d
```

```sh
rescile-vault batch secrets.batch
```

## Architecture

```
   Password ──► Argon2id ──► Master Key ──► HKDF ──┬─► Auth Key  ────► Server (auth)
                                                   └─► KEK ──────────► Wraps the client's X25519 private key

   Collection key (random, AES-256) ──► AES-GCM ──► Secret blobs (per cipher entry)
                                    │
                                    └─► X25519 + HKDF + AES-GCM ──► Per-client wrapped key
                                                                    (stored on server)
```

Server-side, every secret is just an opaque `AES‑GCM(nonce ‖ ct ‖ tag)` blob
keyed by `BLAKE3(collection ‖ name ‖ per-session-salt)`. Collection
membership is implemented by wrapping the symmetric collection key for each
member's public key.

## Development

```sh
# Nix users
nix-shell                  # drops you into the dev shell with rustup, zig, node, etc.

# Common tasks
just                       # list all recipes
just build                 # cargo build --release
just build-static          # static musl build via Zig
just test                  # cargo test
just install               # build-static + install to /usr/local/bin
```

### Cutting a release

```sh
just release 0.2.0         # bump Cargo.toml, commit
# → open PR, merge to main
just mark-release 0.2.0 release   # or `pre` for a prerelease
# → pushes the tag; the GitHub Actions workflow builds the binaries
#   and uploads them as release assets
```

## License

Licensed under the Apache License, Version 2.0. See `LICENSE` for the full
text, or <https://www.apache.org/licenses/LICENSE-2.0>.
