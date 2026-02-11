# Metrics

## Quickstart
For a high level quickstart guide, please refer to [Monitoring](../../l1/running/monitoring.md).

## Ethereum Metrics Exporter

We use the [Ethereum Metrics Exporter](https://github.com/ethpandaops/ethereum-metrics-exporter), a Prometheus metrics exporter for Ethereum execution and consensus nodes, as an additional tool to gather metrics during L1 execution. The exporter uses the prometheus data source to create a Grafana dashboard and display the metrics.

## L1 Metrics Dashboard

We provide a pre-configured Grafana dashboard to monitor Ethrex L1 nodes. For detailed information on the provided dashboard, see our [L1 Dashboard document](./dashboards.md).

### Running the execution node on other networks with metrics enabled

As shown in [Monitoring](../../l1/running/monitoring.md) `docker-compose` is used to bundle prometheus and grafana services, the `*overrides` files define the ports and mounts the prometheus' configuration file.
If a new dashboard is designed, it can be mounted only in that `*overrides` file.
A consensus node must be running for the syncing to work.

To run the execution node on any network with metrics, the next steps should be followed:
1. Build the `ethrex` binary for the network you want (see node options in [CLI Commands](../../CLI.md#cli-commands)) with the `metrics` feature enabled.
2. Enable metrics by using the `--metrics` flag when starting the node.
3. Set the `--metrics.port` cli arg of the ethrex binary to match the port defined in `metrics/provisioning/prometheus/prometheus_l1_sync_docker.yaml`, which is `3701` right now.
4. Run the docker containers:

    ```bash
    cd metrics

    docker compose -f docker-compose-metrics.yaml -f docker-compose-metrics-l1.overrides.yaml up
    ```
