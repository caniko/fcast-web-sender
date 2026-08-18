# Store listing

**Name:** FCast Web Sender

**Summary:** Send selected web media to a trusted local FCast receiver.

**Description:**

FCast Web Sender finds media on the current tab and sends the URL you choose
to a local FCast receiver. Discovery and transport stay on your computer.

This extension requires the native companion on Linux or Windows. After you
install the extension, download the companion from
https://github.com/caniko/fcast-web-sender/releases and run `install.sh` or
`install.ps1`. macOS is not included in 0.2.0.

Credentialed HTTPS media can use a short-lived Cookie or Authorization lease
after you grant that site access. DRM is not supported.

**Privacy policy:** https://github.com/caniko/fcast-web-sender/blob/trunk/docs/privacy.md

**Support:** https://github.com/caniko/fcast-web-sender/issues

**Single purpose:** send user-selected web media to a user-trusted local FCast receiver.

**Permissions justification:**

- `nativeMessaging`: talk to the local companion
- `activeTab` / `scripting`: detect media on the tab you opened the popup on
- `storage`: remember settings
- `cookies` + optional host access: build a lease for credentialed HTTPS media you chose to send
- `webRequest`: observe Authorization on the active tab after you grant that origin

Screenshots: capture the popup with a trusted receiver and with the companion-missing state after those UIs exist.
