"""Load and validate `multisync.toml` into pairs.

Everything a typo could break is checked here, before any container starts:
unknown keys, clients and networks, duplicate names, clashing host ports.
"""

from __future__ import annotations

import os
import re
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any, Optional

try:
    import tomllib
except ModuleNotFoundError:  # Python < 3.11
    import tomli as tomllib

from .clients import CL_CLIENTS, EL_CLIENTS, ClClient, Client, ElClient
from .networks import NETWORKS, Network

EL_RPC_PORT_BASE = 8545
CL_API_PORT_BASE = 5052
NAME_PATTERN = re.compile(r"[a-z0-9-]+")
DEFAULT_BLOCK_PROCESSING_DURATION = 22 * 60  # seconds

PAIR_KEYS = {
    "network": str,
    "el": str,
    "cl": str,
    "name": str,
    "enabled": bool,
    "sync_timeout": int,
    "block_processing_duration": int,
    "el_rpc_port": int,
    "el_metrics_port": int,
    "cl_api_port": int,
    "cl_metrics_port": int,
}
HOST_PORT_KEYS = ("el_rpc_port", "el_metrics_port", "cl_api_port", "cl_metrics_port")
POSITIVE_KEYS = ("sync_timeout", "block_processing_duration")
REQUIRED_PAIR_KEYS = ("network", "el", "cl")
OVERRIDE_KEYS = {"image": str, "pull_policy": str, "extra_args": list, "env": dict}


class ConfigError(Exception):
    """The config is invalid; the message says how."""


@dataclass
class ClientOverride:
    image: Optional[str] = None
    pull_policy: Optional[str] = None
    extra_args: list[str] = field(default_factory=list)
    env: dict[str, str] = field(default_factory=dict)


@dataclass
class Pair:
    name: str
    network: Network
    el: ElClient
    cl: ClClient
    enabled: bool = True
    sync_timeout: Optional[int] = None  # minutes
    block_processing_duration: Optional[int] = None  # seconds
    el_rpc_port: int = 0
    el_metrics_port: Optional[int] = None
    cl_api_port: int = 0
    cl_metrics_port: Optional[int] = None

    @property
    def el_container(self) -> str:
        return f"{self.el.name}-{self.name}"

    @property
    def cl_container(self) -> str:
        return f"{self.cl.name}-{self.name}"

    @property
    def jwt_service(self) -> str:
        return f"setup-jwt-{self.name}"

    @property
    def secrets_volume(self) -> str:
        return f"secrets-{self.name}"

    @property
    def label(self) -> str:
        return f"{self.name}: {self.el.name}+{self.cl.name}"

    def host_ports(self) -> list[tuple[str, int]]:
        ports = [("el_rpc_port", self.el_rpc_port), ("cl_api_port", self.cl_api_port)]
        if self.el_metrics_port is not None:
            ports.append(("el_metrics_port", self.el_metrics_port))
        if self.cl_metrics_port is not None:
            ports.append(("cl_metrics_port", self.cl_metrics_port))
        return ports


@dataclass
class Config:
    pairs: list[Pair]
    overrides: dict[str, ClientOverride]

    def override(self, client_name: str) -> ClientOverride:
        return self.overrides.get(client_name, ClientOverride())


def load(path) -> Config:
    path = Path(path)
    try:
        with path.open("rb") as f:
            data = tomllib.load(f)
    except FileNotFoundError:
        raise ConfigError(f"{path}: file not found") from None
    except tomllib.TOMLDecodeError as e:
        raise ConfigError(f"{path}: {e}") from None
    return parse(data)


def parse(data: dict[str, Any]) -> Config:
    unknown = set(data) - {"pair", "clients"}
    if unknown:
        raise ConfigError(f"unknown top-level keys: {', '.join(sorted(unknown))}")
    raw_pairs = data.get("pair", [])
    if not isinstance(raw_pairs, list) or not raw_pairs:
        raise ConfigError("the config needs at least one [[pair]]")
    pairs = [_parse_pair(raw, index) for index, raw in enumerate(raw_pairs)]
    first_seen: dict[str, int] = {}
    for index, p in enumerate(pairs):
        if p.name in first_seen:
            raise ConfigError(
                f"pair name '{p.name}' is used twice (pairs {first_seen[p.name] + 1} and {index + 1}); "
                "give one of them an explicit name"
            )
        first_seen[p.name] = index
    return Config(pairs=pairs, overrides=_parse_overrides(data.get("clients", {})))


def select(config: Config, only: Optional[str] = None) -> list[Pair]:
    """The pairs to run: those named in `only` (comma-separated), else every enabled pair."""
    names = [name.strip() for name in (only or "").split(",") if name.strip()]
    if names:
        known = [p.name for p in config.pairs]
        unknown = [name for name in names if name not in known]
        if unknown:
            raise ConfigError(f"unknown pair names: {', '.join(unknown)} (known: {', '.join(known)})")
        selected = [p for p in config.pairs if p.name in names]
    else:
        selected = [p for p in config.pairs if p.enabled]
    if not selected:
        raise ConfigError("no pairs selected: enable a pair or name one in MULTISYNC_NETWORKS")
    _check_ports(selected)
    return selected


def default_block_processing_duration() -> int:
    """Window length in seconds when a pair sets none: `BLOCK_PROCESSING_DURATION`, read at call time."""
    return int(os.environ.get("BLOCK_PROCESSING_DURATION", DEFAULT_BLOCK_PROCESSING_DURATION))


def check_windows(pairs: list[Pair], default_duration: int) -> None:
    """The end-of-window finality check needs a window of at least two epochs."""
    for p in pairs:
        duration = p.block_processing_duration if p.block_processing_duration is not None else default_duration
        minimum = 2 * p.network.epoch_seconds
        if duration < minimum:
            raise ConfigError(
                f"pair {p.name}: block_processing_duration {duration}s is under two {p.network.name} "
                f"epochs ({minimum}s), too short to expect finality to advance"
            )


def _parse_pair(raw: Any, index: int) -> Pair:
    where = f"pair {index + 1}"
    if not isinstance(raw, dict):
        raise ConfigError(f"{where}: expected a table")
    _check_keys(raw, PAIR_KEYS, where)
    for key in REQUIRED_PAIR_KEYS:
        if key not in raw:
            raise ConfigError(f"{where}: missing '{key}'")
    network = NETWORKS.get(raw["network"])
    if network is None:
        raise ConfigError(f"{where}: unknown network '{raw['network']}' (known: {', '.join(sorted(NETWORKS))})")
    el = _client(raw["el"], "el", where)
    cl = _client(raw["cl"], "cl", where)
    for client in (el, cl):
        if not client.can_run(network):
            raise ConfigError(f"{where}: {client.name} has no built-in {network.name} and the network publishes no files")
    name = raw.get("name", network.name)
    if not NAME_PATTERN.fullmatch(name):
        raise ConfigError(f"{where}: name '{name}' must match [a-z0-9-]+")
    for key in HOST_PORT_KEYS:
        if key in raw and not 1 <= raw[key] <= 65535:
            raise ConfigError(f"{where}: '{key}' must be between 1 and 65535")
    for key in POSITIVE_KEYS:
        if key in raw and raw[key] <= 0:
            raise ConfigError(f"{where}: '{key}' must be a positive integer")
    return Pair(
        name=name,
        network=network,
        el=el,
        cl=cl,
        enabled=raw.get("enabled", True),
        sync_timeout=raw.get("sync_timeout"),
        block_processing_duration=raw.get("block_processing_duration"),
        el_rpc_port=raw.get("el_rpc_port", EL_RPC_PORT_BASE + index),
        el_metrics_port=raw.get("el_metrics_port"),
        cl_api_port=raw.get("cl_api_port", CL_API_PORT_BASE + index),
        cl_metrics_port=raw.get("cl_metrics_port"),
    )


def _client(name: str, role: str, where: str) -> Client:
    registry, other_role, other = (EL_CLIENTS, "cl", CL_CLIENTS) if role == "el" else (CL_CLIENTS, "el", EL_CLIENTS)
    if name in registry:
        return registry[name]
    if name in other:
        raise ConfigError(f"{where}: '{name}' is a {other_role} client, not an {role} client")
    raise ConfigError(f"{where}: unknown {role} '{name}' (known: {', '.join(sorted(registry))})")


def _check_keys(raw: dict, allowed: dict[str, type], where: str) -> None:
    unknown = set(raw) - set(allowed)
    if unknown:
        raise ConfigError(f"{where}: unknown keys: {', '.join(sorted(unknown))}")
    for key, value in raw.items():
        expected = allowed[key]
        # bool is a subclass of int; `true` is not a port.
        if not isinstance(value, expected) or (expected is int and isinstance(value, bool)):
            raise ConfigError(f"{where}: '{key}' must be of type {expected.__name__}")


def _parse_overrides(raw: Any) -> dict[str, ClientOverride]:
    if not isinstance(raw, dict):
        raise ConfigError("[clients] must be a table")
    overrides = {}
    for name, table in raw.items():
        where = f"clients.{name}"
        if name not in EL_CLIENTS and name not in CL_CLIENTS:
            raise ConfigError(f"{where}: unknown client")
        if not isinstance(table, dict):
            raise ConfigError(f"{where}: expected a table")
        _check_keys(table, OVERRIDE_KEYS, where)
        extra_args = table.get("extra_args", [])
        if not all(isinstance(arg, str) for arg in extra_args):
            raise ConfigError(f"{where}: 'extra_args' must be a list of strings")
        env = table.get("env", {})
        if not all(isinstance(value, str) for value in env.values()):
            raise ConfigError(f"{where}: 'env' values must be strings")
        overrides[name] = ClientOverride(
            image=table.get("image"),
            pull_policy=table.get("pull_policy"),
            extra_args=list(extra_args),
            env=dict(env),
        )
    return overrides


def _check_ports(pairs: list[Pair]) -> None:
    owner: dict[int, str] = {}
    for p in pairs:
        for key, port in p.host_ports():
            if port in owner:
                raise ConfigError(f"host port {port} is used by {owner[port]} and by {p.name}.{key}")
            owner[port] = f"{p.name}.{key}"
