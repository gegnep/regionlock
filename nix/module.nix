# NixOS module for regionlock. Consumed as a flake input:
#   imports = [ inputs.regionlock.nixosModules.regionlock ];
# `self` supplies the default package so no overlay is required.
self:
{
  config,
  lib,
  pkgs,
  ...
}:
let
  cfg = config.programs.regionlock;
  tomlFormat = pkgs.formats.toml { };
in
{
  options.programs.regionlock = {
    enable = lib.mkEnableOption "regionlock, a server picker for Steam Datagram Relay games";

    package = lib.mkOption {
      type = lib.types.package;
      default = self.packages.${pkgs.stdenv.hostPlatform.system}.regionlock;
      defaultText = lib.literalExpression "regionlock.packages.\${system}.regionlock";
      description = "The regionlock package to use.";
    };

    persist = lib.mkOption {
      type = lib.types.bool;
      default = false;
      description = ''
        Apply the configured blocklist at boot via a systemd oneshot and
        remove it on shutdown. Requires {option}`programs.regionlock.settings`
        to be set. The module owns the unit, so the applier detects
        module management and never calls `systemctl` itself.
      '';
    };

    settings = lib.mkOption {
      type = tomlFormat.type;
      default = { };
      example = lib.literalExpression ''
        {
          default_game = "deadlock";
          games.deadlock.desired = [ "fra" "ams" "waw" ];
        }
      '';
      description = ''
        Contents of {file}`/etc/regionlock/config.toml`. Matches the
        config schema documented in the project SPEC (default_game,
        apply_mode, escalator, home_pop, per-game desired blocklists and
        presets).
      '';
    };
  };

  config = lib.mkIf cfg.enable {
    assertions = [
      {
        assertion = !cfg.persist || cfg.settings != { };
        message = "programs.regionlock.persist requires programs.regionlock.settings to be set.";
      }
    ];

    environment.systemPackages = [ cfg.package ];

    # The packaged polkit action ships under share/polkit-1/actions and is
    # picked up because the package is a system package.
    security.polkit.enable = true;

    environment.etc."regionlock/config.toml" = lib.mkIf (cfg.settings != { }) {
      source = tomlFormat.generate "regionlock-config.toml" cfg.settings;
    };

    # The module defines the boot unit itself (store-managed FragmentPath),
    # which the applier detects to skip imperative systemctl. Ordered after
    # the network so a plain online apply resolves the current feed; no /etc
    # feed snapshot is needed on NixOS.
    systemd.services.regionlock = lib.mkIf cfg.persist {
      description = "regionlock: apply the configured firewall blocklist";
      after = [
        "network-online.target"
        "nftables.service"
      ];
      wants = [ "network-online.target" ];
      wantedBy = [ "multi-user.target" ];
      serviceConfig = {
        Type = "oneshot";
        RemainAfterExit = true;
        ExecStart = "${lib.getExe cfg.package} apply --yes --config /etc/regionlock/config.toml";
        ExecStop = "${lib.getExe cfg.package} teardown --yes";
      };
    };
  };
}
