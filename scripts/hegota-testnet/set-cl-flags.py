#!/usr/bin/env python3
"""Print the `docker run` that recreates a stopped kurtosis beacon-node container from a
docker-commit snapshot of itself, identical in every inspectable respect, with extra flags
appended to its command line.

The beacon-node counterpart of set-el-flags.py, which is ethrex-only in its post-checks.
Usage, one node at a time (see docs/hegota-testnet-upgrading.md, "Changing a beacon node's
flags in place"):

  docker stop -t 60 <container> && docker commit <container> cl-N-snapshot:<tag>
  docker rename <container> <container>.old
  set-cl-flags.py <container>.old cl-N-snapshot:<tag> --supernode | bash
  # verify from the node (/eth/v1/node/identity), then: docker rm <container>.old

The command line is one shell string ("exec lighthouse beacon_node ..."), and a flag has to
go inside it: appended as its own argv element, `sh -c` reads it as $0 and the beacon node
never sees it, while `docker inspect` happily shows it. This script appends inside the
string and strips any stray element an earlier attempt left after it."""
import json, shlex, subprocess, sys
old_name, snapshot, *extra = sys.argv[1:]
c = json.loads(subprocess.check_output(["docker", "inspect", old_name]))[0]
cfg, hc, nets = c["Config"], c["HostConfig"], c["NetworkSettings"]["Networks"]
(nname, nc), = nets.items()
cmd = list(cfg.get("Cmd") or [])
# kurtosis runs lighthouse as one shell string ("exec lighthouse beacon_node ..."), so a
# flag has to be appended inside that string; a separate argv element never reaches it.
# Find the shell string wherever it sits ("exec lighthouse ..." as Cmd[0], or Cmd[1] after
# a "-c" when the entrypoint is sh) and append inside it. Then drop any stray flag that an
# earlier attempt left as its own element after the string: sh -c reads that as $0 and the
# beacon node never sees it.
idx = next(i for i, a in enumerate(cmd) if a.startswith("exec "))
for f in extra:
    if f not in cmd[idx].split():
        cmd[idx] = cmd[idx] + " " + f
cmd = cmd[:idx + 1] + [a for a in cmd[idx + 1:] if a not in extra]
name = old_name.removesuffix(".old")
args = ["docker", "run", "-d", "--name", name, "--hostname", cfg["Hostname"],
        "--network", nname, "--ip", nc["IPAddress"],
        "--restart", hc["RestartPolicy"]["Name"] or "no"]
for a in nc.get("Aliases") or []:
    args += ["--network-alias", a]
for k, v in (cfg.get("Labels") or {}).items():
    args += ["--label", f"{k}={v}"]
for m in c["Mounts"]:
    args += ["-v", f'{m["Source"]}:{m["Destination"]}']
for port, binds in (hc.get("PortBindings") or {}).items():
    for b in binds or []:
        args += ["-p", f'{b.get("HostIp") or "0.0.0.0"}:{b["HostPort"]}:{port}']
for e in cfg.get("Env") or []:
    args += ["-e", e]
ep = cfg.get("Entrypoint") or []
if ep:
    args += ["--entrypoint", ep[0]]
args += [snapshot] + ep[1:] + cmd
print("mounts:", [m["Destination"] for m in c["Mounts"]], file=sys.stderr)
print("cmd tail:", cmd[-3:], file=sys.stderr)
print(" ".join(shlex.quote(a) for a in args))
