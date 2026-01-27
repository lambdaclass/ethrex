# Testing Memory Profiling Integration

## Prerequisites

1. Ethrex built with profiling:
   ```bash
   cargo build --features jemalloc_profiling --release
   ```

2. Monitoring stack deployed:
   ```bash
   cd ~/Code/monitoring-stack
   make deploy-alloy  # or appropriate deployment command
   ```

## Manual Testing

### Test 1: Endpoint Accessibility

```bash
# Start ethrex
./target/release/ethrex

# In another terminal, test the endpoint
curl -v http://localhost:8545/debug/pprof/heap -o heap.pprof

# Verify it's a valid pprof file
file heap.pprof
# Should show: gzip compressed data or similar

# Try to parse it with pprof
go tool pprof heap.pprof
```

Expected: Should see memory allocation data

### Test 2: Alloy Scraping

```bash
# Check Alloy is running
systemctl status alloy

# Check Alloy logs for ethrex scraping
journalctl -u alloy -f | grep ethrex

# Should see logs like:
# "scraping profile" component=pyroscope.scrape.ethrex_memory target=localhost:8545
```

### Test 3: Data in Pyroscope

1. Open Grafana UI
2. Navigate to Pyroscope
3. Select service: `ethrex`
4. Select profile type: `memory`
5. Verify profile data appears

Expected: Should see memory allocation flamegraphs

## Troubleshooting

### Endpoint returns 501 NOT IMPLEMENTED

Cause: Binary not built with `jemalloc_profiling` feature

Fix:
```bash
cargo clean
cargo build --features jemalloc_profiling --release
```

### No data in Pyroscope

Cause: Alloy not scraping or connection issue

Debug:
```bash
# Check Alloy can reach ethrex
curl http://localhost:8545/debug/pprof/heap

# Check Alloy config
cat /etc/alloy/config.alloy | grep -A 20 ethrex_memory

# Check Alloy status
systemctl status alloy
journalctl -u alloy -n 100
```

### Profile data looks incorrect

Cause: May need to adjust sampling rate

Fix: In `cmd/ethrex/ethrex.rs:33`, adjust `lg_prof_sample`:
```rust
pub static malloc_conf: &[u8] = b"prof:true,prof_active:true,lg_prof_sample:19\0";
//                                                                         ^^
// Lower number = more frequent sampling = more detail (default: 19 = ~512KB)
// 18 = ~256KB, 17 = ~128KB (more overhead)
```
