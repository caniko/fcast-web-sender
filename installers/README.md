# Companion installers

`native-host.json.in` is the Chromium template and `native-host-firefox.json.in`
is the Firefox template. The published Chromium ID is
`jhcjfdafmhagmemmjbfnbjdnclkonkbh`. Firefox uses `fcast-web-sender@caniko.com`.

## Linux

```sh
./install.sh /path/to/fcast-companion
```

Copies the binary to `~/.local/bin/fcast-companion` and writes native-host JSON
for Chrome, Chromium, and Firefox.

## Windows

```powershell
powershell -File install.ps1 -Binary .\fcast-companion.exe
```

Copies the binary to `%LOCALAPPDATA%\fcast-web-sender` and registers `HKCU`
native-messaging keys for Chrome and Firefox.

## macOS

Not shipped in 0.2.0.
