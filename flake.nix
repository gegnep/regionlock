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
      packages.${system} = {
        regionlock = pkgs.callPackage ./nix/package.nix { };
        default = self.packages.${system}.regionlock;
      };

      # `nix flake check` builds the package, whose doCheck runs the full
      # test suite (including nft --check where nft can init netlink).
      checks.${system}.regionlock = self.packages.${system}.regionlock;

      # Consume as a flake input:
      #   imports = [ inputs.regionlock.nixosModules.regionlock ];
      # The module closes over `self` for its default package (no overlay
      # needed). `overlays.default` is offered for those who prefer
      # `pkgs.regionlock`.
      nixosModules.regionlock = import ./nix/module.nix self;

      overlays.default = _final: prev: {
        regionlock = prev.callPackage ./nix/package.nix { };
      };

      devShells.${system}.default = pkgs.mkShell {
        packages = with pkgs; [
          # toolchain
          cargo
          rustc
          clippy
          rustfmt
          rust-analyzer
          cargo-nextest

          # nft is needed for the env-gated ruleset validation
          # (REGIONLOCK_NFT_CHECK=1) and the privileged e2e verify. Note:
          # `nft --check` still needs netlink cache init, so it does NOT
          # run unprivileged on this host; golden tests byte-compare
          # instead and the check runs during privileged verification.
          nftables
        ];

        # rust-analyzer needs the std sources from nixpkgs rustc.
        env.RUST_SRC_PATH = "${pkgs.rustPlatform.rustLibSrc}";
      };
    };
}
