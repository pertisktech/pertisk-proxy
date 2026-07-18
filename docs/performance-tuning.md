# Performance tuning

The admin UI shows the running values under **Dashboard → Performance tuning**
and **Settings → Performance tuning**. Treat those values as authoritative:
configuration files only show what was requested, while the dashboard shows
what the process and Linux kernel are actually using.

## Recommended production baseline

Use the packaged performance mode:

```ini
PERTISK_PROXY_MODE=performance
```

The mode scales Pingora and Tokio workers to available CPUs, increases listener
parallelism and backlog, expands connection pools, and selects the
performance-oriented HTTP/3 defaults.

The RPM and DEB install `/etc/sysctl.d/99-pertisk-proxy.conf`, which raises:

- UDP/TCP socket buffer ceilings
- socket, NIC, and SYN backlogs
- the ephemeral port range
- TCP BBR congestion control with the `fq` queue discipline

The package post-install script applies the file. Apply it manually after an
edit with:

```bash
sysctl -p /etc/sysctl.d/99-pertisk-proxy.conf
```

## CPU affinity

CPU IDs depend on the host, so packages do not force an affinity. Inspect the
available CPUs and install the example drop-in:

```bash
nproc
lscpu -e=CPU,NODE,CORE,ONLINE
mkdir -p /etc/systemd/system/pertisk-proxy.service.d
cp /usr/share/pertisk-proxy/cpu-affinity.conf.example \
  /etc/systemd/system/pertisk-proxy.service.d/cpu-affinity.conf
```

Edit `CPUAffinity=` so every listed CPU exists. On a shared host, reserve
separate CPUs for the proxy, load generator, and upstream. On a small dedicated
host, using all available CPUs is reasonable.

```bash
systemctl daemon-reload
systemctl restart pertisk-proxy
systemctl show pertisk-proxy -p CPUAffinity -p LimitNOFILE -p Environment
```

## Runtime and connection overrides

Normally, performance mode defaults should be benchmarked before overriding
individual values.

```ini
# Runtime
PERTISK_WORKER_THREADS=8
PERTISK_MAX_BLOCKING_THREADS=512
PERTISK_PINGORA_THREADS=8
PERTISK_PINGORA_LISTENER_TASKS=4
PERTISK_TCP_LISTEN_BACKLOG=8192

# HTTP/3 to upstream pool
PERTISK_H3_UPSTREAM_POOL_MAX_IDLE=256
PERTISK_H3_UPSTREAM_POOL_IDLE_TIMEOUT_SECS=120
PERTISK_H3_UPSTREAM_TCP_KEEPALIVE_SECS=60

# QUIC transport
PERTISK_HTTP3_MAX_STREAMS=1024
PERTISK_HTTP3_STREAM_RECEIVE_WINDOW=8388608
PERTISK_HTTP3_CONN_RECEIVE_WINDOW=67108864
PERTISK_HTTP3_IDLE_TIMEOUT_SECS=300
```

`PERTISK_HTTP3_CC_ALGORITHM` applies to the tokio-quiche backend. Quinn manages
its congestion controller through Quinn's transport implementation.

Default builds use Quinn. Route-level / SQLite `http3` options are stored for
compatibility and are consumed by tokio-quiche; they are **not** the effective
Quinn transport. The admin Settings page shows Quinn's effective env-driven
values under **Effective HTTP/3 / QUIC**.

## UDP offload

On Linux, both supported HTTP/3 stacks use UDP offload when the kernel and NIC
support it:

- Quinn uses `quinn-udp`, which probes GSO and GRO and falls back safely.
- tokio-quiche's listener applies `UDP_SEGMENT`, `UDP_GRO`, and pacing socket
  capabilities where supported.

No environment flag is required. Verify NIC features with:

```bash
ethtool -k eth0 | grep -E 'segmentation|receive-offload'
```

## Benchmarking

Benchmark from a load generator in the same region as the proxy. With a fixed
number of virtual users, throughput is constrained by network round-trip time,
so results from different regions are not directly comparable.

Measure:

- requests per second at a fixed concurrency
- p50, p95, p99, and p99.9 latency
- proxy CPU and resident memory
- active connections and upstream errors
- HTTP/2 and HTTP/3 separately
- small responses, large responses, connection storms, and packet loss

Increase concurrency until proxy CPU saturates to measure server capacity.
For latency comparisons, keep client-to-proxy and proxy-to-upstream RTT equal.
