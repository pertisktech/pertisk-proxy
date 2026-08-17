# pertisk-tunnel

First-party reverse tunnel for [pertisk-proxy](../README.md).

| Binary | Role |
|--------|------|
| `pertisk-tunnel-server` | Runs on the VPS; QUIC UDP control; opens `127.0.0.1:<port>` |
| `pertisk-tunnel-client` | Runs locally; dials the VPS with a shared token |

See [docs/tunnel.md](../docs/tunnel.md) for setup with Sites + ACME.

```bash
cargo build -p pertisk-tunnel-server -p pertisk-tunnel-client --release
# or: make tunnel
```

Examples: [examples/](examples/).
