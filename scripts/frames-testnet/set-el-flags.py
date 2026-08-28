#!/usr/bin/env python3
"""Append ethrex flags to a running kurtosis EL container, preserving its identity.

A container's command line cannot be edited, and every route kurtosis offers for
changing one either does nothing or destroys the node's data (see
`docs/frames-testnet-upgrading.md` §"Changing a node's flags in place"). This
recreates the container from a `docker commit` snapshot of itself instead, so the
writable layer survives: the chain database, and `chain-8141/node.key` — and with
the node key the enode, which is published in `bootnodes.txt`.

Everything else is read back from `docker inspect` rather than hand-transcribed:
name, hostname, network, static IP, network aliases, labels (including the
`traefik.*` routing labels kurtosis' reverse proxy needs), published host ports,
volume binds, restart policy and entrypoint. `docker commit` carries the image
config, so only the command line changes.

Run it on one node at a time, verify, and only then remove the `<container>.old`
container it leaves behind — while that container exists, two containers share the
same `com.kurtosistech.guid` label and `kurtosis enclave inspect` reports the
service STOPPED even though it is running.

The flags to append come last, after a `--` separator: every ethrex flag starts
with a dash, and without the separator the argument parser reads them as options
of this script rather than values for it.

Usage:
  set-el-flags.py <container> <rpc-port> -- --mempool.max-verify-gas=500000
  set-el-flags.py <container> <rpc-port> --dry-run -- --foo=1 --bar=2
"""
import argparse
import json
import shlex
import subprocess
import sys
import time

SNAPSHOT_PREFIX = "el-flag-snapshot"
STOP_TIMEOUT_SECONDS = 60
STARTUP_POLL_ATTEMPTS = 30


def sh(*args, check=True):
    return subprocess.run(args, check=check, capture_output=True, text=True).stdout.strip()


def inspect(name):
    return json.loads(sh("docker", "inspect", name))[0]


def rpc(port, method):
    """Call a local JSON-RPC method, returning None on any failure.

    Used both before the swap (where the node is up) and while polling for it to
    come back (where every kind of failure is expected), so it never raises.
    """
    out = sh("curl", "-s", "-m", "10", "-X", "POST", "-H", "content-type: application/json",
             "--data", json.dumps({"jsonrpc": "2.0", "method": method, "params": [], "id": 1}),
             f"http://127.0.0.1:{port}", check=False)
    try:
        return json.loads(out).get("result")
    except (json.JSONDecodeError, AttributeError):
        return None


def build_run_args(name, config, host_config, network, snapshot, cmd):
    net_name, net_config = network
    args = ["docker", "run", "-d",
            "--name", name,
            "--hostname", config["Hostname"],
            "--network", net_name,
            "--ip", net_config["IPAddress"],
            "--label-file", f"/tmp/{name}.labels",
            "--restart", host_config["RestartPolicy"]["Name"] or "no",
            "--entrypoint", config["Entrypoint"][0]]
    for alias in net_config.get("Aliases") or []:
        args += ["--network-alias", alias]
    for bind in host_config["Binds"] or []:
        args += ["-v", bind]
    for container_port, bindings in (host_config["PortBindings"] or {}).items():
        for binding in bindings:
            args += ["-p", f"{binding['HostIp']}:{binding['HostPort']}:{container_port}"]
    return args + [snapshot] + cmd


def main():
    parser = argparse.ArgumentParser(description=__doc__,
                                     formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("container", help="the EL container name, e.g. el-1-ethrex-lighthouse--<uuid>")
    parser.add_argument("rpc_port", help="the host port that container's 8545 is published on")
    parser.add_argument("flags", nargs="+", metavar="FLAG",
                        help="flags to append, exactly as ethrex takes them; put them after "
                             "a `--` separator so they are not read as options of this script")
    parser.add_argument("--dry-run", action="store_true",
                        help="print the docker run command that would be used and exit")
    opts = parser.parse_args()

    name, port, new_flags = opts.container, opts.rpc_port, opts.flags
    container = inspect(name)
    config, host_config = container["Config"], container["HostConfig"]
    cmd = config["Cmd"]
    network = next(iter(container["NetworkSettings"]["Networks"].items()))

    # A flag ethrex already has must not be appended a second time: a repeated flag
    # either errors or silently keeps one of the two values, and which one is not
    # something this script should be deciding.
    for flag in new_flags:
        stem = flag.split("=", 1)[0]
        if any(arg.split("=", 1)[0] == stem for arg in cmd):
            sys.exit(f"{name}: {stem} is already on the command line; refusing to duplicate it")

    snapshot = f"{SNAPSHOT_PREFIX}:{name.split('--')[0]}"
    new_cmd = cmd + new_flags

    if opts.dry_run:
        print("would stop, commit to", snapshot, "and run:")
        print(" ", " ".join(shlex.quote(a) for a in
                            build_run_args(name, config, host_config, network, snapshot, new_cmd)))
        return

    pre_enode = (rpc(port, "admin_nodeInfo") or {}).get("enode")
    if not pre_enode:
        sys.exit("refusing to proceed: could not read the current enode over RPC, so "
                 "there would be no way to tell afterwards whether it survived")
    print(f"[pre ] head={rpc(port,'eth_blockNumber')} peers={rpc(port,'net_peerCount')}")
    print(f"[pre ] enode={pre_enode}")

    with open(f"/tmp/{name}.labels", "w") as labels:
        for key, value in config["Labels"].items():
            labels.write(f"{key}={value}\n")

    print(f"[stop] {name}")
    sh("docker", "stop", "-t", str(STOP_TIMEOUT_SECONDS), name)
    exit_code = inspect(name)["State"]["ExitCode"]
    print(f"[stop] exit code {exit_code}"
          + ("  (clean)" if exit_code == 0 else "  (NOT clean — the store may need recovery)"))

    print(f"[snap] committing to {snapshot}")
    sh("docker", "commit", name, snapshot)
    sh("docker", "rename", name, name + ".old")

    print(f"[run ] appending: {' '.join(new_flags)}")
    sh(*build_run_args(name, config, host_config, network, snapshot, new_cmd))

    for _ in range(STARTUP_POLL_ATTEMPTS):
        time.sleep(2)
        if rpc(port, "eth_blockNumber"):
            break

    post = inspect(name)
    post_enode = (rpc(port, "admin_nodeInfo") or {}).get("enode")
    print(f"[post] status={post['State']['Status']} head={rpc(port,'eth_blockNumber')} "
          f"peers={rpc(port,'net_peerCount')} syncing={rpc(port,'eth_syncing')}")
    print(f"[post] enode preserved: {post_enode == pre_enode}")
    print(f"[post] flags applied: {all(f in post['Config']['Cmd'] for f in new_flags)}")
    print(f"[next] verify the head against the other nodes, then `docker rm {name}.old` — "
          "until it is gone, kurtosis reports this service STOPPED")


main()
