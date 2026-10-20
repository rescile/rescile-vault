{ pkgs }:
let
  arch = if pkgs.stdenv.hostPlatform.isAarch64 then "aarch64" else "x86_64";
  target = "${arch}-unknown-linux-musl";
  envTarget = pkgs.lib.toUpper (builtins.replaceStrings [ "-" ] [ "_" ] target);
  musl = if arch == "aarch64" then pkgs.pkgsStatic else pkgs.pkgsCross.musl64;
  muslCC = musl.stdenv.cc;
  linker = "${muslCC}/bin/${muslCC.targetPrefix}gcc";
  muslAR = muslCC.bintools;
  muslOpenSSL = musl.openssl.override { static = true; };
  staticTss = openssl: json: tss: tss.overrideAttrs (old: {
    buildInputs = [ openssl json ];
    configureFlags = (old.configureFlags or [ ]) ++ [
      "--enable-static" "--disable-shared" "--enable-nodl" "--disable-fapi"
      "--disable-policy" "--disable-tcti-libtpms" "--disable-tcti-cmd"
      "--disable-tcti-mssim" "--disable-tcti-pcap" "--disable-doxygen-doc"
      "--disable-unit" "--disable-integration"
    ];
  });
  muslTss = staticTss muslOpenSSL musl.json_c musl.tpm2-tss;
  common = [
    pkgs.rustup pkgs.cargo-edit pkgs.just pkgs.git pkgs.gh pkgs.jq pkgs.python3
    pkgs.nodejs_24 pkgs.pkg-config pkgs.binutils pkgs.file pkgs.coreutils
  ];
  localHomes = ''
    export RUSTUP_HOME="$PWD/.rustup"
    export CARGO_HOME="$PWD/.cargo"
    export PATH="$CARGO_HOME/bin:$PATH"
  '';
in
{
  default = pkgs.mkShell {
    packages = common ++ [ pkgs.openssl pkgs.openssl.dev ];
    shellHook = localHomes;
  };
  frontend = pkgs.mkShell {
    packages = [ pkgs.nodejs_24 pkgs.just ];
  };
  release-tools = pkgs.mkShell {
    packages = [ pkgs.coreutils pkgs.findutils pkgs.git pkgs.just pkgs.gh ];
  };
  release-musl = pkgs.mkShell ({
    packages = common ++ [ muslCC muslAR muslTss muslTss.dev muslOpenSSL muslOpenSSL.dev musl.json_c ];
    CARGO_TARGET_DIR = "target/nix-release-musl";
    RESCILE_RELEASE_TARGET = target;
    RESCILE_RELEASE_PROFILE = "release-musl";
    shellHook = localHomes;
  } // builtins.listToAttrs [
    { name = "CARGO_TARGET_${envTarget}_LINKER"; value = linker; }
    { name = "CC_${builtins.replaceStrings [ "-" ] [ "_" ] target}"; value = linker; }
    { name = "AR_${builtins.replaceStrings [ "-" ] [ "_" ] target}"; value = "${muslAR}/bin/${muslAR.targetPrefix}ar"; }
    { name = "PKG_CONFIG_ALLOW_CROSS_${builtins.replaceStrings [ "-" ] [ "_" ] target}"; value = "1"; }
    { name = "PKG_CONFIG_LIBDIR_${builtins.replaceStrings [ "-" ] [ "_" ] target}"; value =
        "${muslTss.dev}/lib/pkgconfig:${muslOpenSSL.dev}/lib/pkgconfig:${musl.json_c.dev}/lib/pkgconfig"; }
    { name = "${envTarget}_OPENSSL_STATIC"; value = "1"; }
    { name = "${envTarget}_OPENSSL_LIB_DIR"; value = "${muslOpenSSL.out}/lib"; }
    { name = "${envTarget}_OPENSSL_INCLUDE_DIR"; value = "${muslOpenSSL.dev}/include"; }
    { name = "CARGO_TARGET_${envTarget}_RUSTFLAGS"; value =
        "-L native=${muslTss.out}/lib -l static=tss2-tcti-device -l static=tss2-tcti-swtpm -C link-arg=${muslTss.out}/lib/libtss2-sys.a -C link-arg=${muslTss.out}/lib/libtss2-mu.a"; }
  ]);
}
