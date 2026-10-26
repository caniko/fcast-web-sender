# Home Manager module: installs the native companion, registers it as a
# native messaging host for Chromium, Brave and Firefox, and exposes the
# unpacked extensions under $XDG_DATA_HOME/fcast-web-sender/extensions.
self: {
  config,
  lib,
  pkgs,
  ...
}: let
  cfg = config.programs.fcast-web-sender;
  packages = self.packages.${pkgs.stdenv.hostPlatform.system};
  chromiumExtensionId = "jhcjfdafmhagmemmjbfnbjdnclkonkbh";
  firefoxExtensionId = "fcast-web-sender@caniko.com";
  nativeHostName = "com.caniko.fcast_web_sender";

  manifest = allowed:
    builtins.toJSON ({
        name = nativeHostName;
        description = "FCast Web Sender native companion";
        path = "${cfg.package}/bin/fcast-companion";
        type = "stdio";
      }
      // allowed);
  chromiumManifest = manifest {allowed_origins = ["chrome-extension://${chromiumExtensionId}/"];};
  firefoxManifest = manifest {allowed_extensions = [firefoxExtensionId];};
in {
  options.programs.fcast-web-sender = {
    enable = lib.mkEnableOption "FCast Web Sender native companion and browser hosts";
    package = lib.mkOption {
      type = lib.types.package;
      default = packages.default;
      defaultText = lib.literalExpression "fcast-web-sender.packages.\${system}.default";
      description = "Package providing bin/fcast-companion.";
    };
    chromiumExtension = lib.mkOption {
      type = lib.types.path;
      default = "${packages.extensions}/extensions/chromium";
      defaultText = lib.literalExpression ''"''${fcast-web-sender.packages.''${system}.extensions}/extensions/chromium"'';
      description = "Unpacked Chromium extension directory.";
    };
    firefoxExtension = lib.mkOption {
      type = lib.types.path;
      default = "${packages.extensions}/extensions/firefox";
      defaultText = lib.literalExpression ''"''${fcast-web-sender.packages.''${system}.extensions}/extensions/firefox"'';
      description = "Unpacked Firefox extension directory.";
    };
  };

  config = lib.mkIf cfg.enable {
    home.packages = [cfg.package];

    xdg.configFile = {
      "chromium/NativeMessagingHosts/${nativeHostName}.json".text = chromiumManifest;
      "BraveSoftware/Brave-Browser/NativeMessagingHosts/${nativeHostName}.json".text = chromiumManifest;
    };

    xdg.dataFile = {
      "fcast-web-sender/extensions/chromium".source = cfg.chromiumExtension;
      "fcast-web-sender/extensions/firefox".source = cfg.firefoxExtension;
    };

    home.file.".mozilla/native-messaging-hosts/${nativeHostName}.json".text = firefoxManifest;
  };
}
