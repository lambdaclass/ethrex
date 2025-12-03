# Performance Guide

## Introduction

This guide introduces the art done measuring performance of ethrex and other clients.

## Tooling

### OS Tools

We use a family of tools, most Linux focused, some portable.

`perf` software is part of `linux-tools-common` package.

#### Cache Measurements

#### Contention Measurements

#### Scheduling Measurements

We record schedule latency to analyze bottlenecks. The command is:

```shell
perf sched record -- sleep 10
```

We discovered that `merkle_worker_1` was taking more execution time than the other ones.



#### IO Measurements


```shell
perf record -e block:block_rq_issue -e block:block_rq_complete -a sleep 120
```


```shell
iostat -x 1
```
#### Integrated Tools

For convenience, we often use `samply`, an interactive tool compatible with both Linux and MacOS. Our typical usage is:

```shell
samply record -p $(pgrep ethrex) --unstable-presymbolicate --profile-name <some_name> --reuse-threads --fold-recursive-prefix --cswitch-markers -s
```

We're currently disfavouring it due to it sometimes producing hard to understand artifacts, e.g. sometimes what are potentially preemptions look like a long sample of a CPU-bound operation or blocking operations seem to be called by synchronization instructions.

### In-Process Tools

Ethrex can be built with support for profiling based on the `pprof-rs` crate. It can profile different stages of execution:
- `regenerate_head`: the initial execution made to help the diff layers catch up with the current head;
- `fullsync`: the block execution phase of synchronization with the chain, happens either after a snapsync or after the client is shut down and started again;
- `block_execution`: a profile per block executed after catching up.

This mode produces two files for each stage (with one per block in the last stage):
- A flamegraph in SVG format, showing samples separate by thread;
- A protobuf-serialized pprof file that can be loaded with:

```shell
pprof -http=localhost:8080 profile-regenerate_head.pb
```

To use this, you need to build ethrex with the `profiling` feature enabled and with frame pointers:
```shell
RUSTFLAGS="-Cforce-frame-pointers" cargo +nigthly -Zbuild-std b --profile release-with-debug --bin ethrex --features profiling
```

The use of nightly is due to lack of frame pointers in the stdlib as shipped by `rustup`.

<!-- TODO: see if we can detect the use of prebuilt stdlib and disable the `frame-pointers` feature; that leads to worse profiles but technically works as well -->

## Comparing with Other Clients

While clients don't all work the same way, most often their behavior should be similar enough for comparison amongst them to be a useful way to spot places where Ethrex might be abnormally slow, thus finding current bottlenecks. We use the following methods to build profiles to compare to.

### Reth

First, we need to compile `reth` in profiling mode to get the proper symbols, as well as force the compiler to use frame pointers for better quality reports.

Following their performance guide, we also use the native CPU and a few optimization-related features:
```shell
RUSTFLAGS="-Cforce-frame-pointers -Ctarget-cpu=native" cargo +nightly -Zbuild-std --profile profiling --features jemalloc,asm-keccak
```

Once that's done, you should run `reth` normally, pointing to the appropriate binary:
```shell
./target/profiling/reth node --chain mainnet --http --http.addr 0.0.0.0 --http.port 8545 --http.api eth,web3,net,debug,trace,txpool --authrpc.port 8551 --discovery.port 30300 --port 30300 --metrics 100.90.204.15:6060 --authrpc.jwtsecret /secrets/jwt.hex --db.read-transaction-timeout 0 --rpc.eth-proof-window 100000
```

Now you can attach `perf` for CPU profiling or `offcpu-bcc` for off-CPU profiling, using `c++filt` to demangle symbols:
```shell
sudo /sbin/offcputime-bpfcc -df -p $(pgrep reth) | c++filt > offcpu.folded
sudo perf record -p $(pgrep reth) -g
sudo perf script | c++filt > oncpu.stacks
```
<!--
TODO: try with rustfilt instead
https://crates.io/crates/rustfilt
-->

Then you can build flamegraphs with either inferno or Brendan Gregg's scripts:
```shell
inferno-collapse-perf < oncpu.stacks > oncpu.folded
inferno-flamegraph --title="On-CPU Time Flame Graph" --countname=us < oncpu.folded  > oncpu.svg
inferno-flamegraph --colors=io --title="Off-CPU Time Flame Graph" --countname=us < offcpu.folded  > offcpu.svg
```

## References

* [perf Examples, by Brendan Gregg](https://www.brendangregg.com/perf.html)
* [Fast by Friday - Why Kernel Superpowers are Essent, Brendan Gregg](https://www.brendangregg.com/Slides/KernelRecipes2023_FastByFriday.pdf)




<!--
# Notes
- Debian installs bcc-bpftools in /sbin
- Many commands, notably block io related ones, don't seem to work due to functions being inlined
- A lot of IO seems to come from Lighthouse actually, we should try to see if having it separate gives more stability
- bpftrace mostly works and implements an AWK-like language, so it might be ideal to script some tooling
- The straightforward off-CPU flamegraph from Brendann Gregg works
```bash
sudo /sbin/offcputime-bpfcc -df -p $(pgrep ethrex) > out.stacks
inferno-flamegraph --colors=io --title="Off-CPU Time Flame Graph" --countname=us < out.stacks  > out.svg
```
- tracing-flame seems to produce folded stacks correctly, but it's been abandonned for 4 years and its docs are down, with our current instrumentation the data produced is not particularly useful either
- perf one-liners -> cache and TLB behavior is BAD (~30% and ~20% misses), tried to enable huge pages with no success
- Build with frame pointers and run profiling (`RUSTFLAGS="-Cforce-frame-pointers"`)

# TODO
- hotspot
- flamescope

Para profilear Reth:
Recompilar con profile profiling, buildeando la std y con frame pointers (últimos dos pasos opcionales pero mejoran los resultados), para que libmdbx compile hay que instalar libclang-dev;
Instalar bpfcc-tools, linux-headers-$(uname -r), linux-perf;
Para offcpu: sudo /sbin/offcputime-bpfcc -df -p $(pgrep reth) > offcpu.stacks;
Para oncpu: sudo perf -p $(pgrep reth) -g; sudo perf script > oncpu.stacks;
En ambos casos pipear a c++filt para demanglear los nombres;
Usar inferno para armar los flamegraphs.
Command line de reth  (ajustar a la ubicación del binario):
reth node --chain mainnet --http --http.addr 0.0.0.0 --http.port 8545 --http.api eth,web3,net,debug,trace,txpool --authrpc.port 8551 --discovery.port 30300 --port 30300 --metrics 100.90.204.15:6060 --authrpc.jwtsecret /secrets/jwt.hex --db.read-transaction-timeout 0 --rpc.eth-proof-window 100000

reth node --chain mainnet --http --http.addr 0.0.0.0 --http.port 8545 --http.api eth,web3,net,debug,trace,txpool --authrpc.port 8551 --discovery.port 30300 --port 30300 --metrics 100.90.204.15:6060 --authrpc.jwtsecret /secrets/jwt.hex --db.read-transaction-timeout 0 --rpc.eth-proof-window 100000 
-->
