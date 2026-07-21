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

      # M6 adds packages.${system}.regionlock and nixosModules.regionlock here.
      # The nixos flake consumes those; this repo never installs system files.
    };
}
