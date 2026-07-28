# Mock receiver

`cargo run -p fcast-mock-receiver` provides a deterministic line-oriented
fixture receiver for integration tests. It acknowledges JSON commands with an
FCast v4 protocol marker and never opens a network listener by default.
