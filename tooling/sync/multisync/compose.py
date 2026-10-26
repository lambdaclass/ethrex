"""Generate the multisync compose file from the config.

Compose accepts JSON, so the generated file is JSON and needs no YAML library.
Service names equal container names: `{el}-{pair}`, `{cl}-{pair}`,
`setup-jwt-{pair}` and `setup-config-{network}`.

    python3 -m multisync.compose generate [--config F] [--only a,b] [--out F] [--image-override CLIENT=IMAGE]
    python3 -m multisync.compose services --pair NAME [--role el|cl] [--config F]
"""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path
from typing import Optional

from .clients import DATA_DIR, Client
from .config import ClientOverride, Config, ConfigError, Pair, load, select
from .networks import NETWORK_CONFIG_DIR

DEFAULT_CONFIG = "multisync.toml"
DEFAULT_OUTPUT = "docker-compose.multisync.json"
JWT_SCRIPT = 'apk add openssl && openssl rand -hex 32 | tr -d "\\n" | tee /secrets/jwt.hex'
COMPLETED = {"condition": "service_completed_successfully"}


def build(config: Config, pairs: list[Pair], image_overrides: Optional[dict[str, str]] = None) -> dict:
    """Compose document for `pairs`. `image_overrides` maps a client to a local image."""
    image_overrides = image_overrides or {}
    services: dict[str, dict] = {}
    volumes: set[str] = set()
    for pair in pairs:
        services[pair.jwt_service] = {
            "image": "alpine",
            "volumes": [f"{pair.secrets_volume}:/secrets"],
            "command": ["sh", "-c", JWT_SCRIPT],
        }
        volumes.add(pair.secrets_volume)

        engine_url = f"http://{pair.el_container}:{pair.el.engine_port}"
        el_command = pair.el.command(pair.network, config.override(pair.el.name).extra_args)
        cl_command = pair.cl.command(pair.network, engine_url, config.override(pair.cl.name).extra_args)
        el_ports = [(pair.el_rpc_port, pair.el.rpc_port), (pair.el_metrics_port, pair.el.metrics_port)]
        cl_ports = [(pair.cl_api_port, pair.cl.api_port), (pair.cl_metrics_port, pair.cl.metrics_port)]
        for client, container, command, ports in (
            (pair.el, pair.el_container, el_command, el_ports),
            (pair.cl, pair.cl_container, cl_command, cl_ports),
        ):
            services[container] = _client_service(
                pair, client, container, command, ports,
                config.override(client.name), image_overrides.get(client.name),
            )
            volumes.add(container)
            if client.uses_network_files(pair.network):
                services[pair.network.setup_service_name] = pair.network.setup_service()
                volumes.add(pair.network.config_volume)
    return {"services": services, "volumes": {name: {} for name in sorted(volumes)}}


def _client_service(
    pair: Pair,
    client: Client,
    container: str,
    command: list[str],
    ports: list[tuple[Optional[int], int]],
    override: ClientOverride,
    image_override: Optional[str],
) -> dict:
    volumes = [f"{pair.secrets_volume}:/secrets", f"{container}:{DATA_DIR}"]
    depends_on = {pair.jwt_service: COMPLETED}
    if client.uses_network_files(pair.network):
        volumes.append(f"{pair.network.config_volume}:{NETWORK_CONFIG_DIR}:ro")
        depends_on[pair.network.setup_service_name] = COMPLETED
    service = {
        "image": image_override or override.image or client.default_image,
        "pull_policy": "never" if image_override else (override.pull_policy or client.default_pull_policy),
        "container_name": container,
        "restart": "unless-stopped",
        "volumes": volumes,
        "ports": [f"{host}:{inner}" for host, inner in ports if host is not None],
        "command": command,
        "depends_on": depends_on,
    }
    entrypoint = client.entrypoint(pair.network)
    if entrypoint:
        service["entrypoint"] = entrypoint
    environment = {**client.environment, **override.env}
    if environment:
        service["environment"] = environment
    if client.ulimits_nofile:
        service["ulimits"] = {"nofile": client.ulimits_nofile}
    if client.stop_signal:
        service["stop_signal"] = client.stop_signal
    if client.stop_grace_period:
        service["stop_grace_period"] = client.stop_grace_period
    return service


def generate(config_path, out_path, only: Optional[str] = None, image_overrides: Optional[dict[str, str]] = None) -> list[Pair]:
    """Write the compose file for the selected pairs and return them."""
    config = load(config_path)
    pairs = select(config, only)
    document = build(config, pairs, image_overrides)
    Path(out_path).write_text(json.dumps(document, indent=2) + "\n")
    return pairs


def services(config_path, pair_name: str, role: Optional[str] = None) -> list[str]:
    """Service names of one pair, or of one of its clients."""
    for pair in load(config_path).pairs:
        if pair.name == pair_name:
            if role == "el":
                return [pair.el_container]
            if role == "cl":
                return [pair.cl_container]
            return [pair.el_container, pair.cl_container]
    raise ConfigError(f"unknown pair '{pair_name}'")


def _image_override(text: str) -> tuple[str, str]:
    client, sep, image = text.partition("=")
    if not sep or not client or not image:
        raise argparse.ArgumentTypeError("expected CLIENT=IMAGE")
    return client, image


def main(argv: Optional[list[str]] = None) -> int:
    parser = argparse.ArgumentParser(prog="python3 -m multisync.compose", description="Multisync compose generator")
    commands = parser.add_subparsers(dest="command", required=True)
    gen = commands.add_parser("generate", help="write the compose file for the selected pairs")
    gen.add_argument("--config", default=DEFAULT_CONFIG)
    gen.add_argument("--only", default="", help="comma-separated pair names (default: every enabled pair)")
    gen.add_argument("--out", default=DEFAULT_OUTPUT)
    gen.add_argument("--image-override", action="append", default=[], type=_image_override,
                     metavar="CLIENT=IMAGE", help="run a local image for CLIENT (pull_policy never)")
    svc = commands.add_parser("services", help="print one pair's service names")
    svc.add_argument("--config", default=DEFAULT_CONFIG)
    svc.add_argument("--pair", required=True)
    svc.add_argument("--role", choices=("el", "cl"))
    args = parser.parse_args(argv)
    try:
        if args.command == "generate":
            pairs = generate(args.config, args.out, args.only, dict(args.image_override))
            print(f"{args.out}: {', '.join(p.label for p in pairs)}")
        else:
            print(" ".join(services(args.config, args.pair, args.role)))
    except ConfigError as e:
        print(f"error: {e}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
