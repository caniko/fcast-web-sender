# Decision 0001: local control plane

Status: accepted

Discovery, trust, FCast transport, and optional media fetching stay in a local
Rust companion. The extension talks to it through bounded native-messaging
JSON. This keeps browser permissions narrow, avoids sending media bytes
through a control channel, and gives one place to enforce receiver identity
and SSRF policy.
