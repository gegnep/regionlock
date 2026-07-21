{
  description = "regionlock - server picker for Steam Datagram Relay games";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

  outputs =
    { self, nixpkgs }:
    let
      system = "x86_64-linux";
      pkgs = nixpkgs.legacyPackages.${system};
    in
    {
      devShells.${system}.default = pkgs.mkShell {
        packages = with pkgs; [
          # toolchain
          cargo
          rustc
          clippy
          rustfmt
          rust-analyzer
          cargo-nextest

          # `nft --check -f` validates generated rulesets without root.
          # Golden-file tests in core depend on it.
          nftables
        ];

        # rust-analyzer needs the std sources from nixpkgs rustc.
        env.RUST_SRC_PATH = "${pkgs.rustPlatform.rustLibSrc}";
      };

      # M6 adds packages.${system}.regionlock and nixosModules.regionlock here.
      # The nixos flake consumes those; this repo never installs system files.
    };
}
