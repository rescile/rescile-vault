{
  description = "rescile-vault development and Linux release toolchains";
  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

  outputs = { self, nixpkgs }: {
    devShells = builtins.listToAttrs (map (system: {
      name = system;
      value = import ./nix/shells.nix { pkgs = nixpkgs.legacyPackages.${system}; };
    }) [ "x86_64-linux" "aarch64-linux" ]);
  };
}
