# Monitoring and Metrics

Ethrex exposes metrics in Prometheus format on port `9090` by default. But the easiest way to monitor your node is to use the provided Docker Compose stack, which includes Prometheus and Grafana preconfigured. For that we are currently using port `3701`, this will match the default in the future but for now if running the containers we expected to have the ethrex metrics exposed on port `3701`.

## Quickstart: Monitoring Stack with Docker Compose

1. **Clone the repository:**

   ```sh
   git clone https://github.com/lambdaclass/ethrex.git
   cd ethrex/metrics
   ```

2. **Start the monitoring stack:**
   ```sh
   # Optional: if you have updated from a previous version, stop first the docker compose.
   # docker compose -f docker-compose-metrics.yaml -f docker-compose-metrics-l1.overrides.yaml down
   docker compose -f docker-compose-metrics.yaml -f docker-compose-metrics-l1.overrides.yaml up -d
   ```

_**Note:** You might want to restart the docker containers in case of an update from a previous ethrex version to make sure the latest provisioned configurations are applied:_

3. **Run ethrex with metrics enabled:**

   Make sure to start ethrex with the `--metrics` flag and set the port to `3701`:

   ```sh
   ethrex --authrpc.jwtsecret ./secrets/jwt.hex --network hoodi --metrics --metrics.port 3701
   ```

This will launch Prometheus and Grafana, already set up to scrape ethrex metrics.

**Note: We depend on `ethereum-metrics-exporter` for some key metrics to define variables on the Grafana dashboards. For it to work properly we need the consensus client to expose its RPC endpoints. For example if you are running lighthouse you may need to add `--http` and `--http-address 0.0.0.0` flags to it before the dashboards pick up all metrics. This won't be needed in the near future**

## Logs

Ethrex logs are written to stdout by default. To enable file logging, you must specify the `--log.dir` argument, with this you'll be able to have Promtail collect the logs and send them to Grafana Loki for log visualization.

- **Promtail Configuration:** `metrics/provisioning/promtail/promtail.yaml`

The promtail configuration expects by default that logs are stored in `./logs` (relative to the repo root). To correctly see the logs in Grafana, ensure that Promtail can access the logs directory:

- If running via **Docker**, ensure you map a volume to the log directory and pass `--log.dir` to the container.
- If running **standalone**, use `--log.dir ./logs` when running ethrex.

```bash
ethrex --log.dir ./logs ...
```

If you choose to use a different directory, you must set the `ETHREX_LOGS_DIR` environment variable when running the metrics stack to point to your custom logs directory.

```bash
ETHREX_LOGS_DIR=/path/to/your/logs docker compose -f docker-compose-metrics.yaml -f docker-compose-metrics-l1.overrides.yaml up
```

You can view the logs in Grafana by navigating to the [logs row](../../developers/l1/dashboards.md#logs) in our dashboard.

### Running Docker Container Manually

If you run the `ethrex` Docker container manually (e.g., `docker run ...`) or use a custom `docker-compose.yaml` outside of this repository, you must ensure the logs are accessible to the monitoring stack.

The `ethrex` container writes logs to the directory specified by `--log.dir`. You should mount this directory to a location on your host machine that Promtail can access.

Example:
```bash
docker run -d \
  --name ethrex \
  -v $(pwd)/logs:/data/logs \
  ghcr.io/lambdaclass/ethrex:main \
  --datadir /data \
  --log.dir /data/logs
```

If you are using the provided monitoring stack in `metrics/`, it expects logs to be in the `logs` directory at the root of the repository (or `../logs` relative to the `metrics` folder). Ensure your volume mount matches this expectation or update the Promtail volume configuration.

## Accessing Metrics and Dashboards

- **Prometheus:** [http://localhost:9091](http://localhost:9091)
- **Grafana:** [http://localhost:3001](http://localhost:3001)
  - Default login: `admin` / `admin`
  - Prometheus is preconfigured as a data source
  - Example dashboards are included in the repo

Metrics from ethrex will be available at `http://localhost:3701/metrics` in Prometheus format if you followed [step 3](#run-ethrex-with-metrics-enabled).

For detailed information on the provided Grafana dashboards, see our [L1 Dashboard document](../../developers/l1/dashboards.md).

## Custom Configuration

Your ethrex setup may differ from the default configuration. Check your endpoints at `provisioning/prometheus/prometheus_l1_sync_docker.yaml`.

Also if you have a centralized Prometheus or Grafana setup, you can adapt the provided configuration files to fit your environment, or even stop the docker containers that run Prometheus and/or Grafana leaving only the additional `ethereum-metrics-exporter` running alongside ethrex to export the metrics to your existing monitoring stack.

```sh
docker compose -f docker-compose-metrics.yaml -f docker-compose-metrics-l1.overrides.yaml up -d ethereum-metrics-exporter 
```

---

For manual setup or more details, see the [Prometheus documentation](https://prometheus.io/docs/introduction/overview/) and [Grafana documentation](https://grafana.com/docs/).
