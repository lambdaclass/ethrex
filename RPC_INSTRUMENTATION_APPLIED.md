# RPC Instrumentation - Middleware Approach with Namespace Separation

## What Was Done

Added per-method profiling for RPC and Engine API calls using a middleware pattern with explicit namespace separation.

## Changes

### 1. `crates/blockchain/metrics/profiling.rs`
- Added `namespace` label to histogram: `&["namespace", "function_name"]`
- Uses wrapper structs `MethodName` and `Namespace` to disambiguate span extensions
- Extracts both `namespace` and `method` fields from spans
- Falls back to module path-based detection if namespace not explicitly set
- Supports explicit namespace override via span fields

### 2. `crates/networking/rpc/rpc.rs`
- Updated middleware function to accept namespace parameter:
```rust
async fn instrumented_call<T: RpcHandler>(
    namespace: &str,
    method: &str, 
    req: &RpcRequest, 
    context: RpcApiContext
) -> Result<Value, RpcErr> 
{
    let span = tracing::trace_span!("rpc_call", namespace = %namespace, method = %method);
    let _enter = span.enter();
    T::call(req, context).await
}
```
- Applied to **60+ RPC methods** with namespace separation:
  - **35 eth_\* methods** → `namespace="rpc"`
  - **7 debug_\* methods** → `namespace="rpc"`
  - **18 engine_\* methods** → `namespace="engine"`

### 3. `metrics/provisioning/grafana/dashboards/common_dashboards/ethrex_l1_perf.json`
- Added **Engine API Performance** row (id: 110) with 4 panels:
  - Engine API Time Breakdown (piechart)
  - Top 15 Slowest Engine API Methods (table with 4s threshold)
  - Engine API Request Rate by Method (timeseries)
  - Engine API Latency Percentiles (timeseries with 4s warning line)
- Updated existing **RPC Performance** row (id: 100) with 4 panels:
  - RPC Time Breakdown (piechart)
  - Top 15 Slowest RPC Methods (table)
  - RPC Request Rate by Method (timeseries)
  - RPC Latency Percentiles (timeseries)
- Updated **Block Execution Breakdown** panels to filter out RPC/Engine metrics using `namespace!="rpc"` and `namespace!="engine"`

## Metrics Generated

```promql
# RPC API metrics (eth_*, debug_*)
function_duration_seconds{namespace="rpc", function_name="eth_blockNumber"}
function_duration_seconds{namespace="rpc", function_name="debug_traceTransaction"}

# Engine API metrics (engine_*)
function_duration_seconds{namespace="engine", function_name="engine_newPayloadV4"}
function_duration_seconds{namespace="engine", function_name="engine_forkchoiceUpdatedV3"}

# Block execution metrics (unchanged)
function_duration_seconds{namespace="block_processing", function_name="Execute Block"}
```

## Namespace Separation Rationale

**Engine API** is separated from general **RPC API** because:
- Engine API is **critical for validators** - directly interfaces with consensus clients
- **Payload build time** must be < 4s for block proposals (beacon chain slot timing)
- `newPayload`, `forkchoiceUpdated`, and `getPayload` have strict performance requirements
- Different SLA requirements: Engine API failures impact network consensus, RPC API failures impact user experience
- Enables targeted monitoring and alerting for validator operations

## Grafana Queries

```promql
# RPC only
rate(function_duration_seconds_sum{namespace="rpc"}[5m])

# RPC latency percentiles
histogram_quantile(0.99, rate(function_duration_seconds_bucket{namespace="rpc"}[5m]))

# Compare namespaces
sum by (namespace) (rate(function_duration_seconds_sum[5m]))
```

## Testing

```bash
# Start with metrics
cargo run --release -- --metrics-enabled

# Check metrics
curl http://localhost:6060/metrics | grep 'namespace="rpc"'
```
