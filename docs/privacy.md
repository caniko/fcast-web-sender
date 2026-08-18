# Privacy policy

FCast Web Sender is a local sender. The browser extension and native companion
do not include analytics, crash reporting, or a cloud relay.

## What stays on the device

Discovery, receiver trust, and FCast transport run in the companion on your
computer. Media bytes are not sent through the native-messaging channel. The
receiver you confirm by fingerprint fetches the media URL you selected.

Trust records are stored only in a local file
(`~/.config/fcast-web-sender/trust.json` on Linux,
`%APPDATA%\fcast-web-sender\trust.json` on Windows).

## Credentials

Cookie and Authorization values are forwarded only when you send credentialed
HTTPS media to a trusted receiver. The companion holds that lease in memory,
binds it to one receiver fingerprint and origin, expires it within five
minutes, and does not persist it. The extension may observe Authorization
headers on the active tab so a lease can be created after you grant that
site's host permission.

## Native companion

The extension talks to `com.caniko.fcast_web_sender` over native messaging.
Install that companion from the project's GitHub Releases. The companion is
not a browser store package.

## Unsupported

DRM, encrypted media keys, and circumvention are intentionally unsupported.

Contact: the issue tracker at https://github.com/caniko/fcast-web-sender
