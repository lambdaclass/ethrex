# SP1 v6 GPU Benchmark Attempt — RTX 5090

**Date:** 2026-02-12
**Branch:** `bench/sp1-hypercube`
**Server:** `ethrex-gpu-5090-1` (via `ssh admin@ethrex-gpu-5090-1`)
**Outcome:** Failed — SP1 v6.0.0-rc.1 GPU prover incompatible with RTX 5090 (Blackwell)

## Server Environment

| Property | Value |
|----------|-------|
| OS | Ubuntu 24.04.3 LTS |
| GPU | NVIDIA GeForce RTX 5090 (32 GB VRAM) |
| Compute capability | 12.0 (Blackwell) |
| CUDA installed | 13.0 (`/usr/local/cuda-13.0`) |
| SP1 version | 6.0.0-rc.1 |
| `sp1-gpu-server` | Pre-built binary at `~/.sp1/bin/sp1-gpu-server` |

## Benchmark Configuration

| Parameter | Value |
|-----------|-------|
| Backend | SP1 |
| GPU | Yes |
| Tx type | erc20 |
| Tx per account | 50 |
| Batches to prove | 15 |
| Endless mode | Yes |

## What Happened

### 1. Build succeeded

`cargo build --release --features l2,l2-sql,sp1,gpu` compiled in ~12 minutes with minor warnings (unused import, dead code). No build errors.

### 2. Localnet started successfully

L1 (Docker), contract deployment (with SP1 verifier), L2 node, and ERC20 load test all started without issues. The L2 was producing blocks and committing batches.

### 3. Prover failed at GPU initialization

#### Issue 1: `libcudart.so.12` not found

```
/home/admin/.sp1/bin/sp1-gpu-server: error while loading shared libraries:
libcudart.so.12: cannot open shared object file: No such file or directory
```

**Cause:** The server has CUDA 13.0 installed (`libcudart.so.13`) but SP1 v6's pre-built `sp1-gpu-server` binary is linked against CUDA 12 (`libcudart.so.12`).

**Fix applied:** Installed `cuda-cudart-12-8` package alongside CUDA 13 and registered via `ldconfig`:

```bash
wget https://developer.download.nvidia.com/compute/cuda/repos/ubuntu2404/x86_64/cuda-keyring_1.1-1_all.deb -O /tmp/cuda-keyring.deb
sudo dpkg -i /tmp/cuda-keyring.deb
sudo apt-get update
sudo apt-get install -y cuda-cudart-12-8
echo /usr/local/cuda-12.8/lib64 | sudo tee /etc/ld.so.conf.d/cuda-12.conf
sudo ldconfig
```

After this fix, `sp1-gpu-server` could load and run.

#### Issue 2: `CUDA_VISIBLE_DEVICES` not propagated

```
CUDA_VISIBLE_DEVICES must be set: NotPresent
```

**Cause:** The SP1 SDK's `CudaProver` builder spawns `sp1-gpu-server` as a child process, but the `CUDA_VISIBLE_DEVICES` environment variable set on the parent (ethrex prover) is not propagated to the child process by the SDK.

**Workaround:** Pre-start `sp1-gpu-server` manually before launching the prover:

```bash
CUDA_VISIBLE_DEVICES=0 ~/.sp1/bin/sp1-gpu-server &
# Then start the prover normally
```

This successfully creates the socket at `/tmp/sp1-cuda-0.sock` and the prover connects to it. A proper fix would require passing the device ID through the SP1 SDK API rather than relying on environment variable inheritance.

#### Issue 3: AllocError crash during proving (fatal)

```
panicked at sp1/slop/crates/tensor/src/inner.rs:28:51:
called `Result::unwrap()` on an `Err` value: AllocError {
  layout: Layout { size: 34809856, align: 4 (1 << 2) }
}
```

**Cause:** After successfully connecting and receiving batch 1 for Groth16 proving, the SP1 GPU prover crashed attempting a 34 MB tensor allocation. This is not a memory shortage (32 GB VRAM available, 0 MB used). The most likely cause is that the pre-built `sp1-gpu-server` binary (v6.0.0-rc.1) does not support compute capability 12.0 (Blackwell / RTX 5090).

**Impact:** After this crash, `sp1-gpu-server` exits and subsequent proving attempts fail with `BrokenPipe`. The prover enters an infinite retry loop:

```
INFO  starting proof generation mode=Groth16
ERROR Proving error: CudaClientError: Failed to write the request:
      Os { code: 32, kind: BrokenPipe, message: "Broken pipe" }
```

**No workaround available.** This requires SP1 to release a `sp1-gpu-server` build compiled for sm_100 (Blackwell).

## Summary of Issues

| # | Issue | Severity | Status |
|---|-------|----------|--------|
| 1 | `libcudart.so.12` missing (CUDA 13 server) | Blocking | **Fixed** — installed `cuda-cudart-12-8` |
| 2 | `CUDA_VISIBLE_DEVICES` not propagated by SDK | Blocking | **Workaround** — pre-start server manually |
| 3 | `AllocError` crash on Blackwell GPU (sm_100) | Fatal | **No fix** — requires SP1 Blackwell support |

## Recommendations

1. **Use a server with Ampere/Hopper GPU** (RTX 3090/4090, A100, H100) for SP1 v6 GPU benchmarks until Blackwell support is added.
2. **Report to Succinct** that `sp1-gpu-server` v6.0.0-rc.1 crashes on RTX 5090 (compute capability 12.0) with an `AllocError` in the tensor allocation path.
3. **CPU proving** is an alternative but will be significantly slower.

## Changes Made to Server

- Installed `cuda-keyring` apt package (NVIDIA CUDA repo for Ubuntu 24.04)
- Installed `cuda-cudart-12-8` package at `/usr/local/cuda-12.8/`
- Added `/etc/ld.so.conf.d/cuda-12.conf` pointing to `/usr/local/cuda-12.8/lib64`
- Ran `ldconfig` to register the new library path
