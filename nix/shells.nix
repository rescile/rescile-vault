{ pkgs }:
let
  musl = pkgs.pkgsCross.musl64;
  muslCC = musl.stdenv.cc;
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
  gnuOpenSSL = pkgs.openssl.override { static = true; };
  gnuTss = staticTss gnuOpenSSL pkgs.json_c pkgs.tpm2-tss;
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
    packages = common;
    shellHook = localHomes;
  };
  frontend = pkgs.mkShell {
    packages = [ pkgs.nodejs_24 pkgs.just ];
  };
  release-tools = pkgs.mkShell {
    packages = [ pkgs.coreutils pkgs.findutils pkgs.git pkgs.just pkgs.gh ];
  };
  release-musl = pkgs.mkShell {
    packages = common ++ [ muslCC muslAR muslTss muslTss.dev muslOpenSSL muslOpenSSL.dev musl.json_c ];
    CARGO_TARGET_DIR = "target/nix-release-musl";
    CARGO_TARGET_X86_64_UNKNOWN_LINUX_MUSL_LINKER = "${muslCC}/bin/${muslCC.targetPrefix}gcc";
    CC_x86_64_unknown_linux_musl = "${muslCC}/bin/${muslCC.targetPrefix}gcc";
    AR_x86_64_unknown_linux_musl = "${muslAR}/bin/${muslAR.targetPrefix}ar";
    PKG_CONFIG_ALLOW_CROSS_x86_64_unknown_linux_musl = "1";
    PKG_CONFIG_LIBDIR_x86_64_unknown_linux_musl =
      "${muslTss.dev}/lib/pkgconfig:${muslOpenSSL.dev}/lib/pkgconfig:${musl.json_c.dev}/lib/pkgconfig";
    X86_64_UNKNOWN_LINUX_MUSL_OPENSSL_STATIC = "1";
    X86_64_UNKNOWN_LINUX_MUSL_OPENSSL_LIB_DIR = "${muslOpenSSL.out}/lib";
    X86_64_UNKNOWN_LINUX_MUSL_OPENSSL_INCLUDE_DIR = "${muslOpenSSL.dev}/include";
    CARGO_TARGET_X86_64_UNKNOWN_LINUX_MUSL_RUSTFLAGS =
      "-L native=${muslTss.out}/lib -l static=tss2-tcti-device -l static=tss2-tcti-swtpm -C link-arg=${muslTss.out}/lib/libtss2-sys.a -C link-arg=${muslTss.out}/lib/libtss2-mu.a";
    RESCILE_RELEASE_PROFILE = "release-musl";
    shellHook = localHomes;
  };
  release-gnu-tpm = pkgs.mkShell {
    packages = common ++ [ pkgs.stdenv.cc gnuTss gnuTss.dev gnuOpenSSL gnuOpenSSL.dev pkgs.json_c ];
    CARGO_TARGET_DIR = "target/nix-release-gnu-tpm";
    LIBRARY_PATH = "${pkgs.glibc.static}/lib";
    PKG_CONFIG_ALL_STATIC = "1";
    PKG_CONFIG_PATH = "${gnuTss.dev}/lib/pkgconfig:${gnuOpenSSL.dev}/lib/pkgconfig:${pkgs.json_c.dev}/lib/pkgconfig";
    OPENSSL_STATIC = "1";
    OPENSSL_LIB_DIR = "${gnuOpenSSL.out}/lib";
    OPENSSL_INCLUDE_DIR = "${gnuOpenSSL.dev}/include";
    RESCILE_RELEASE_PROFILE = "release-gnu-tpm";
    shellHook = ''
      ${localHomes}
      export RUSTFLAGS="-C target-feature=+crt-static -L native=${gnuTss.out}/lib -l static=tss2-tcti-device -l static=tss2-tcti-swtpm -C link-arg=${gnuTss.out}/lib/libtss2-sys.a -C link-arg=${gnuTss.out}/lib/libtss2-mu.a"
    '';
  };
}
