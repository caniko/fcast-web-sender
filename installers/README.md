# Installer inputs

The platform directories contain templates only. `native-host.json.in` is the
Chromium template and `native-host-firefox.json.in` is the Firefox template.
An installer must substitute the real installed companion path and the
published Chromium extension ID `jhcjfdafmhagmemmjbfnbjdnclkonkbh`, validate both values, set executable
permissions, and register the host with the browser's documented
native-messaging location. The Firefox template uses the fixed Gecko ID from
the Firefox manifest. No template is usable until those values are supplied
by the package builder.
