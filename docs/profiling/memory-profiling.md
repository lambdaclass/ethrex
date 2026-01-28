# Memory Profiling with Jemalloc

## Overview

Ethrex supports memory profiling using jemalloc's built-in profiling capabilities. This allows you to collect heap allocation profiles in pprof format for analysis.

## Building with Profiling Enabled

```bash
cargo build --features jemalloc_profiling --release
```

## Endpoints

When built with the `jemalloc_profiling` feature, ethrex exposes the following debug endpoints:

- `GET /debug/pprof/allocs` - Returns heap allocation profile in pprof binary format
- `GET /debug/pprof/allocs/flamegraph` - Returns an SVG flamegraph visualization

These endpoints are only available when:
1. Built with `jemalloc_profiling` feature
2. Running on Linux (jemalloc profiling is Linux-only)
3. Profiling is activated (automatically enabled with the feature)

## Integration with Grafana Pyroscope

The `/debug/pprof/allocs` endpoint is compatible with Grafana Pyroscope and can be scraped using Grafana Alloy's `pyroscope.scrape` component.

See [lambdaclass/monitoring-stack](https://github.com/lambdaclass/monitoring-stack) for Ansible configuration examples.

## Manual Analysis

Download a profile and analyze it with pprof:

```bash
# Download profile
curl http://localhost:8545/debug/pprof/allocs > heap.pprof

# Analyze with go pprof
go tool pprof -http=:8080 heap.pprof
```

## Troubleshooting

If the endpoint returns a 501 NOT IMPLEMENTED error:
- Verify the binary was built with `--features jemalloc_profiling`
- Ensure you're running on Linux (profiling is not available on macOS/Windows)

If the endpoint returns 403 FORBIDDEN:
- Check that profiling is activated (should be automatic with the feature flag)
