#!/usr/bin/env bash
set -euo pipefail

chromium_id="jhcjfdafmhagmemmjbfnbjdnclkonkbh"
here="$(cd "$(dirname "$0")" && pwd)"

find_bin() {
  if [ "${1:-}" != "" ]; then printf '%s\n' "$1"; return; fi
  if [ -x "$here/bin/fcast-companion" ]; then printf '%s\n' "$here/bin/fcast-companion"; return; fi
  if [ -x "$here/fcast-companion" ]; then printf '%s\n' "$here/fcast-companion"; return; fi
  command -v fcast-companion
}

bin="$(find_bin "${1:-}" || true)"
if [ -z "${bin}" ] || [ ! -x "${bin}" ]; then
  echo "usage: install.sh /path/to/fcast-companion" >&2
  exit 1
fi
bin="$(readlink -f "$bin")"
install -D -m 755 "$bin" "$HOME/.local/bin/fcast-companion"
dest="$HOME/.local/bin/fcast-companion"

template_dir="$here"
if [ ! -f "$template_dir/native-host.json.in" ]; then
  template_dir="$(cd "$here/.." && pwd)/linux"
fi

render() {
  sed -e "s|@COMPANION_PATH@|${dest}|g" -e "s|@CHROMIUM_EXTENSION_ID@|${chromium_id}|g" "$1"
}

mkdir -p "$HOME/.config/google-chrome/NativeMessagingHosts" \
  "$HOME/.config/chromium/NativeMessagingHosts" \
  "$HOME/.mozilla/native-messaging-hosts"
render "$template_dir/native-host.json.in" | tee \
  "$HOME/.config/google-chrome/NativeMessagingHosts/com.caniko.fcast_web_sender.json" \
  "$HOME/.config/chromium/NativeMessagingHosts/com.caniko.fcast_web_sender.json" >/dev/null
render "$template_dir/native-host-firefox.json.in" > \
  "$HOME/.mozilla/native-messaging-hosts/com.caniko.fcast_web_sender.json"
echo "installed $dest and registered native host com.caniko.fcast_web_sender"
