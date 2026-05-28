{
  pkgs ? import <nixpkgs> { },
}:

let
  # --- Configuration for Static Linux (musl) Cross-Compilation ---
  # target = "x86_64-unknown-linux-musl";
  target = "x86_64-unknown-linux-gnu";

  # Zig uses a slightly different target triple format than Rust (e.g., no 'unknown' vendor).
  zig_musl_target = "x86_64-linux-musl";
  zig_gnu_target = "x86_64-linux-gnu";

  # Wrapper scripts to use `zig cc` as a linker for different Rust targets.
  # This is needed because the CARGO_..._LINKER env var expects a single executable,
  # but we need to pass `cc` and other arguments to `zig`.
  zig-linker-musl = pkgs.writeShellScriptBin "zig-linker-musl" ''
    #!${pkgs.stdenv.shell}
    exec ${pkgs.zig}/bin/zig cc -target ${zig_musl_target} "$@"
  '';
  zig-linker-gnu = pkgs.writeShellScriptBin "zig-linker-gnu" ''
    #!${pkgs.stdenv.shell}
    # Create a standard dynamically-linked gnu binary. For fully static
    # binaries, the 'musl' target should be used. Forcing static linking
    # with glibc is problematic and breaks build scripts.
    exec ${pkgs.zig}/bin/zig cc -target ${zig_gnu_target} "$@"
  '';

  # --- Wrappers for C Compiler and Archiver ---
  # These wrappers solve two common cross-compilation problems:
  # 1. Build systems (like openssl's) that split CC/AR env vars by spaces,
  #    which fails when the variable contains arguments (e.g., "zig cc -target ...").
  #    By using a wrapper script, the env var is a single executable path.
  # 2. The `cc-rs` crate automatically adds a `--target` flag with the Rust
  #    triple (e.g., `--target=x86_64-unknown-linux-musl`), which Zig's C
  #    compiler does not understand. The CC wrappers strip this argument before
  #    executing `zig cc` with the correct Zig-compatible target triple.
  stripTargetArgScript = ''
    args=()
    for arg in "$@"; do
      case "$arg" in
        --target=*) ;; # Strip --target from `cc-rs`
        *) args+=("$arg") ;;
      esac
    done
  '';

  zig-cc-musl = pkgs.writeShellScriptBin "zig-cc-musl" ''
    #!${pkgs.stdenv.shell}
    ${stripTargetArgScript}
    exec ${pkgs.zig}/bin/zig cc -target ${zig_musl_target} "''${args[@]}"
  '';
  zig-cc-gnu = pkgs.writeShellScriptBin "zig-cc-gnu" ''
    #!${pkgs.stdenv.shell}
    ${stripTargetArgScript}
    exec ${pkgs.zig}/bin/zig cc -target ${zig_gnu_target} "''${args[@]}"
  '';
  zig-ar = pkgs.writeShellScriptBin "zig-ar" ''
    #!${pkgs.stdenv.shell}
    exec ${pkgs.zig}/bin/zig ar "$@"
  '';

in
pkgs.mkShell {
  # Native build inputs for the host development environment.
  # We use `rustup` to manage toolchains and targets declaratively via `rust-toolchain.toml`.
  nativeBuildInputs = [
    pkgs.rustup
    pkgs.just
    pkgs.gh
    pkgs.jq
    pkgs.glow # For pretty-printing the rust-toolchain.toml suggestion
    pkgs.zig # For static compilation
    pkgs.pkg-config
    pkgs.tpm2-tss
    # tpm2-tss has split outputs (out/man/dev); only "out" and "man" are in
    # outputsToInstall by default, so the dev output (which carries tss2-*.pc
    # files) needs to be listed explicitly for pkg-config to find them.
    pkgs.tpm2-tss.dev
    pkgs.openssl
    pkgs.python3
    pkgs.nodejs
    # pkgs.glibc.static # Needed for static gnu linking
    zig-linker-musl
    zig-linker-gnu
    zig-cc-musl
    zig-cc-gnu
    zig-ar
  ];

  # --- Environment Variables for Cargo and Rustc ---

  # Set the default build target for `cargo build`, `cargo run`, etc.
  CARGO_BUILD_TARGET = target;
  BUILD = target;

  # Enable the 'openssl-vendored' feature for all cargo commands (build, test, etc.).
  # This is the environment variable equivalent of `--features openssl-vendored`.
  # It ensures that `cargo test` will also build and link OpenSSL statically.
  # CARGO_BUILD_FEATURES = "openssl-vendored";

  # Tell rustc where to find the C linker for each target using our wrappers.
  # The format is `CARGO_TARGET_<TRIPLE_WITH_UNDERSCORES>_LINKER`.
  CARGO_TARGET_X86_64_UNKNOWN_LINUX_MUSL_LINKER = "zig-linker-musl";
  # CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_LINKER = "zig-linker-gnu";

  # When using Zig as the linker for the `musl` target, Rust's built-in,
  # "self-contained" C runtime objects (like `crt1.o`) conflict with the
  # ones provided by Zig. This flag tells rustc not to link its own CRT
  # objects for this target, delegating that responsibility to Zig and
  # resolving the "duplicate symbol" linker error.
  CARGO_TARGET_X86_64_UNKNOWN_LINUX_MUSL_RUSTFLAGS = "-C link-self-contained=no";

  # --- Environment Variables for `openssl-sys` (vendored) ---
  # Set the C compiler and archiver for the target. This is crucial for the
  # `openssl-sys` crate's build script when using the `vendored` feature,
  # as it needs to compile OpenSSL from source for the target architecture.
  # We use our wrappers to ensure compatibility.
  CC_x86_64_unknown_linux_musl = "zig-cc-musl";
  AR_x86_64_unknown_linux_musl = "zig-ar";
  # CC_x86_64_unknown_linux_gnu = "zig-cc-gnu";
  # AR_x86_64_unknown_linux_gnu = "zig-ar";

  shellHook = ''
        # Set rustup and cargo home to the project directory to avoid polluting ~/.rustup
        export RUSTUP_HOME=$(pwd)/.rustup
        export CARGO_HOME=$(pwd)/.cargo

        # Ensure cargo-installed binaries are in PATH
        export PATH=$CARGO_HOME/bin:$PATH

        # Advise the user on the required `rust-toolchain.toml` configuration.
        if [ ! -f rust-toolchain.toml ]; then
          # Use glow to pretty-print the suggestion if available.
          if command -v glow &> /dev/null; then
            glow - <<'EOF'
    # Recommended `rust-toolchain.toml`
    Your project is missing a `rust-toolchain.toml` file. Create one with the following content to ensure a reproducible build environment for static binaries:
    ```toml
    [toolchain]
    channel = "stable"
    profile = "minimal"
    components = ["rust-analyzer", "rustfmt"]
    targets = ["x86_64-unknown-linux-musl"]
    ```
    EOF
          else
            echo "--> Recommended: Create a 'rust-toolchain.toml' file with targets = [\"''${target}\"]"
          fi
        fi

        # rustup will automatically install the toolchain and target
        # defined in rust-toolchain.toml on first entry.
        echo "Rust cross-compilation environment for ' ''${target}' is ready."
        rustup show

        # Add project-local node_modules to PATH
        export PATH="$PWD/node_modules/.bin/:$PATH"
  '';
}
