# Metrics

## Ethereum Metrics Exporter

We use the [Ethereum Metrics Exporter](https://github.com/ethpandaops/ethereum-metrics-exporter), a Prometheus metrics exporter for Ethereum execution and consensus nodes, to gather metrics during syncing for L1. The exporter uses the prometheus data source to create a Grafana dashboard and display the metrics. For the syncing to work there must be a consensus node running along with the execution node.

Currently we have two make targets to easily start an execution node and a consensus node on either hoodi or holesky, and display the syncing metrics. In both cases we use a lighthouse consensus node.

### Quickstart guide

Make sure you have your docker daemon running.

- **Code Location**: The targets are defined in `tooling/sync/Makefile`.
- **How to Run**:

   ```bash
   # Navigate to tooling/sync directory
   cd tooling/sync

   # Run target for hoodi
   make start-hoodi-metrics-docker

    # Run target for holesky
   make start-holesky-metrics-docker
   ```

To see the dashboards go to [http://localhost:3001](http://localhost:3001). Use “admin” for user and password. Select the Dashboards menu and go to Ethereum Metrics Exporter (Single) to see the exported metrics.

To see the prometheus exported metrics and its respective requests with more detail in case you need to debug go to [http://localhost:9093/metrics](http://localhost:9093/metrics).

### Running the execution node on other networks with metrics enabled

A `docker-compose` is used to bundle prometheus and grafana services, the `*overrides` files define the ports and mounts the prometheus' configuration file.
If a new dashboard is designed, it can be mounted only in that `*overrides` file.
A consensus node must be running for the syncing to work.

To run the execution node on any network with metrics, the next steps should be followed:
1. Build the `ethrex` binary for the network you want (see node options in [CLI Commands](../../CLI.md#cli-commands)) with the `metrics` feature enabled.
2. Enable metrics by using the `--metrics` flag when starting the node.
3. Set the `--metrics.port` cli arg of the ethrex binary to match the port defined in `metrics/provisioning/prometheus/prometheus_l1_sync_docker.yaml`
4. Run the docker containers:

    ```bash
    cd metrics

    docker compose -f docker-compose-metrics.yaml -f docker-compose-metrics-l1.overrides.yaml up
    ```
For more details on running a sync go to `tooling/sync/readme.md`.
