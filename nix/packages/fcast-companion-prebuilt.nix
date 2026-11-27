{
  pkgs,
  harbor-rs,
}: let
  version = "0.1.0";
  fetchReleaseAsset = name: hash:
    pkgs.fetchzip {
      url = "https://github.com/caniko/fcast-web-sender/releases/download/${version}/${name}";
      inherit hash;
      stripRoot = false;
    };
  companion = harbor-rs.lib.mkReleaseBinaryPackage {
    inherit pkgs version;
    pname = "fcast-companion";
    sources.x86_64-linux = fetchReleaseAsset "fcast-companion-${version}-x86_64-linux-musl.tar.gz" "sha256-6ukkD65rT4It6XIB7QKU1QqNEkia/IlsNLE1tlnMkQI=";
    binaries = ["fcast-companion"];
  };
in
  companion.overrideAttrs (old: {
    passthru =
      (old.passthru or {})
      // {
        chromiumExtension = fetchReleaseAsset "fcast-web-sender-chromium-${version}.zip" "sha256-CRBm5wxrpfNHQeTulTNBKhq2cWZQmOXYcXdGHWcu0OQ=";
        firefoxExtension = fetchReleaseAsset "fcast-web-sender-firefox-${version}.zip" "sha256-63x+TqUJhNsokhMygbLnX3zOH6iyFrUV/H55R6xmKgg=";
      };
  })
