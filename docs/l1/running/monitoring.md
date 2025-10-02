# Monitoring and Metrics

Ethrex exposes metrics in Prometheus format on port `9090` by default. The easiest way to monitor your node is to use the provided Docker Compose stack, which includes Prometheus and Grafana preconfigured.

## Quickstart: Monitoring Stack with Docker Compose

1. **Clone the repository:**

   ```sh
   git clone https://github.com/lambdaclass/ethrex.git
   cd ethrex/metrics
   ```

2. **Start the monitoring stack:**
   ```sh
   docker compose -f docker-compose-metrics.yaml -f docker-compose-metrics-l1.overrides.yaml up -d
   ```

This will launch Prometheus and Grafana, already set up to scrape ethrex metrics.

## Accessing Metrics and Dashboards

- **Prometheus:** [http://localhost:9091](http://localhost:9091)
- **Grafana:** [http://localhost:3001](http://localhost:3001)
  - Default login: `admin` / `admin`
  - Prometheus is preconfigured as a data source
  - Example dashboards are included in the repo

Metrics from ethrex will be available at `http://localhost:9090/metrics` in Prometheus format.

## Custom Configuration

Your ethrex setup may differ from the default configuration. Check your endpoints at `provisioning/prometheus/prometheus_l1_sync_docker.yaml`.

---

For manual setup or more details, see the [Prometheus documentation](https://prometheus.io/docs/introduction/overview/) and [Grafana documentation](https://grafana.com/docs/).
