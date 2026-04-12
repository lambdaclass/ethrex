# Metrics

A `docker-compose` is used to bundle prometheus and grafana services, the `*overrides` files define the ports and mounts the prometheus' configuration file.
If a new dashboard is designed, it can be mounted only in that `*overrides` file.

To run the node with metrics, the next steps should be followed:
1. Build the `ethrex` binary with the `metrics` feature enabled.
2. Enable metrics by using the `--metrics` flag when starting the node.
3. Set the `--metrics.port` cli arg of the ethrex binary to match the port defined in `metrics/provisioning/prometheus/prometheus*.yaml`
4. Run the docker containers, example with the L2:

```sh
docker compose -f docker-compose-metrics.yaml -f docker-compose-metrics-l2.overrides.yaml up
```

> [!NOTE]
> The L2's Makefile automatically starts the prometheus and grafana services with `make init` for the L2.


- For the L2 we use the following files in conjunction:
  - `docker-compose-metrics.yaml`
  - `docker-compose-metrics-l2.overrides.yaml`
  - The defaults are:
    - PORT `3702` &rarr; metrics API (used by prometheus)
    - PORT `3802` &rarr; Grafana
      - usr: `admin`
      - pwd: `admin` 
    - PORT `9092` &rarr; Prometheus


### Alerts

An extra `overrides` file is available that enable alerts about the chain status. The alerts notify via Slack when the L2 is not advancing, the mempool is increasing fast (probably meaning that transactions are not getting processed), and if the chain is not advancing in L1.
To enable this feature, add the `docker-compose-metrics-alerts.override.yaml` file to the Docker Compose command. Also, the following environment variables **must** be set up:

- `GRAFANA_SLACK_CHANNEL`: The name of the channel (or user ID) where the will be sent.
- `GRAFANA_SLACK_TOKEN`: A Slack token with write permissions on the desired channel's chat. This token starts with `xoxb-`.

