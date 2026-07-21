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

    security.polkit.enable = true;

    # Install the packaged polkit action (its exec.path already rewritten to
    # this package's applier) where polkit reads unconditionally. Without
    # this the exec.path would not match the store binary and pkexec would
    # fall back to the generic action: no custom message, no auth caching.
    environment.etc."polkit-1/actions/org.pengeg.regionlock.policy".source =
      "${cfg.package}/share/polkit-1/actions/org.pengeg.regionlock.policy";

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
