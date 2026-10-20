{
  pkgs ? import <nixpkgs> { },
}:

let
  # --- Configuration for Static Linux (musl) Cross-Compilation ---
  # target = "x86_64-unknown-linux-musl";
  target = "x86_64-unknown-linux-gnu";

  muslCC = pkgs.pkgsCross.musl64.stdenv.cc;
  muslAR = muslCC.bintools;
  muslCompiler = "${muslCC}/bin/${muslCC.targetPrefix}gcc";
  muslArchiver = "${muslAR}/bin/${muslAR.targetPrefix}ar";
  muslPkgs = pkgs.pkgsCross.musl64;
  muslOpenSSL = muslPkgs.openssl.override { static = true; };
  muslTss = muslPkgs.tpm2-tss.overrideAttrs (old: {
    # No FAPI or dlopen: link ESYS and the device/swtpm transports into the binary.
    buildInputs = [ muslOpenSSL muslPkgs.json_c ];
    configureFlags = (old.configureFlags or [ ]) ++ [
      "--enable-static"
      "--disable-shared"
      "--enable-nodl"
      "--disable-fapi"
      "--disable-policy"
      "--disable-tcti-libtpms"
      "--disable-tcti-cmd"
      "--disable-tcti-mssim"
      "--disable-tcti-pcap"
      "--disable-doxygen-doc"
      "--disable-unit"
      "--disable-integration"
    ];
  });

in
pkgs.mkShell {
  # Native build inputs for the host development environment.
  # We use `rustup` to manage toolchains and targets declaratively via `rust-toolchain.toml`.
  nativeBuildInputs = [
    pkgs.rustup
    pkgs.cargo-edit
    pkgs.just
    pkgs.gh
    pkgs.jq
    pkgs.glow # For pretty-printing the rust-toolchain.toml suggestion
    muslCC # Cross-compiler for native C dependencies and musl linking
    muslAR
    muslTss
    muslTss.dev
    muslOpenSSL
    muslOpenSSL.dev
    pkgs.pkg-config
    pkgs.binutils # readelf verifies that the musl binary is fully static
    pkgs.tpm2-tss
    # tpm2-tss has split outputs (out/man/dev); only "out" and "man" are in
    # outputsToInstall by default, so the dev output (which carries tss2-*.pc
    # files) needs to be listed explicitly for pkg-config to find them.
    pkgs.tpm2-tss.dev
    pkgs.openssl
    pkgs.python3
    pkgs.nodejs
  ];

  # --- Environment Variables for Cargo and Rustc ---

  # Set the default build target for `cargo build`, `cargo run`, etc.
  CARGO_BUILD_TARGET = target;
  BUILD = target;

  # Use the musl cross-toolchain for Rust linking and native C dependencies.
  CARGO_TARGET_X86_64_UNKNOWN_LINUX_MUSL_LINKER = muslCompiler;
  CC_x86_64_unknown_linux_musl = muslCompiler;
  AR_x86_64_unknown_linux_musl = muslArchiver;
  PKG_CONFIG_ALLOW_CROSS_x86_64_unknown_linux_musl = "1";
  PKG_CONFIG_PATH_x86_64_unknown_linux_musl = "";
  PKG_CONFIG_LIBDIR_x86_64_unknown_linux_musl =
    "${muslTss.dev}/lib/pkgconfig:${muslOpenSSL.dev}/lib/pkgconfig:${muslPkgs.json_c.dev}/lib/pkgconfig";
  X86_64_UNKNOWN_LINUX_MUSL_OPENSSL_STATIC = "1";
  X86_64_UNKNOWN_LINUX_MUSL_OPENSSL_LIB_DIR = "${muslOpenSSL.out}/lib";
  X86_64_UNKNOWN_LINUX_MUSL_OPENSSL_INCLUDE_DIR = "${muslOpenSSL.dev}/include";
  # nodl needs device/swtpm TCTIs. Re-link SYS after ESYS: tss-esapi-sys
  # emits SYS before ESYS, which leaves references unresolved with static archives.
  CARGO_TARGET_X86_64_UNKNOWN_LINUX_MUSL_RUSTFLAGS =
    "-L native=${muslTss.out}/lib -l static=tss2-tcti-device -l static=tss2-tcti-swtpm -C link-arg=${muslTss.out}/lib/libtss2-sys.a -C link-arg=${muslTss.out}/lib/libtss2-mu.a";

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
    components = ["rust-analyzer", "rustfmt", "clippy"]
    targets = ["x86_64-unknown-linux-musl"]
    ```
    EOF
          else
            echo "--> Recommended: Create a 'rust-toolchain.toml' file with targets = [\"${target}\"]"
          fi
        fi

        # rustup will automatically install the toolchain and target
        # defined in rust-toolchain.toml on first entry.
        echo "Rust cross-compilation environment for '${target}' is ready."
        rustup show

        # Add project-local node_modules to PATH
        export PATH="$PWD/node_modules/.bin/:$PATH"
  '';
}
