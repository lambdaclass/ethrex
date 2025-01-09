# Metrics

A `docker-compose` is used to bundle prometheus and grafana services, the `*overrides` files define the ports and mounts the prometheus' configuration file.
If a new dashboard is designed just for the L1 or L2, it can be mounted only in that `*overrides` file.

To run the node with metrics, the next steps should be followed:
1. Build the `ethrex` binary with the `metrics` feature enabled.
2. Set the `--metrics.port` cli arg of the ethrex binary to match the port defined in `metrics/provisioning/prometheus/prometheus*.yaml`
3. Run the docker containers, example with the L2:

```sh
docker compose -f docker-compose-metrics.yaml -f docker-compose-metrics-l2.override.yaml up
```

>[!NOTE]
> The L2's Makefile automatically starts the prometheus and grafana services with `make init`. For the L1 used in dev mode and the L2.


- For the L2 we use the following files in conjunction:
  - `docker-compose-metrics.yaml`
  - `docker-compose-metrics-l2.overrides.yaml`
  - The defaults are:
    - PORT `3702` &rarr; metrics API (used by prometheus)
    - PORT `3802` &rarr; Grafana
      - usr: `admin`
      - pwd: `admin` 
    - PORT `9092` &rarr; Prometheus

- For the L1 dev we use the following files in conjunction:
  - `docker-compose-metrics.yaml`
  - `docker-compose-metrics-l1-dev.overrides.yaml`
  - The defaults are:
    - PORT `3701` &rarr; metrics API (used by prometheus)
    - PORT `3801` &rarr; Grafana
      - usr: `admin`
      - pwd: `admin` 
    - PORT `9091` &rarr; Prometheus
