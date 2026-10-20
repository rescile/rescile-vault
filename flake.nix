{
  description = "rescile-vault development and Linux release toolchains";
  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

  outputs = { self, nixpkgs }: {
    devShells.x86_64-linux = import ./nix/shells.nix {
      pkgs = nixpkgs.legacyPackages.x86_64-linux;
    };
  };
}
