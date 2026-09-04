#!/usr/bin/env python3
"""Request the frames testnet's P2P port mappings from the gateway over UPnP.

Only the P2P ports are mapped. Engine authrpc, EL RPC, metrics and beacon REST
are deliberately left unmapped, and the host firewall drops them regardless.

Usage: upnp.py list | add | delete
"""
import re
import sys
import urllib.request
import xml.etree.ElementTree as ET

DESC = "http://192.168.1.1:49652/49652gatedesc.xml"
INTERNAL = "192.168.1.3"

# (external_port, protocol, description)
MAPPINGS = []
for p in (36000, 36007, 36014):
    MAPPINGS += [(p, "TCP", f"frames-el-disc-{p}"), (p, "UDP", f"frames-el-disc-{p}")]
for p in (36200, 36207, 36214):
    MAPPINGS += [(p, "TCP", f"frames-cl-libp2p-{p}"), (p, "UDP", f"frames-cl-quic-{p}")]
for p in (36201, 36208, 36215):
    MAPPINGS += [(p, "UDP", f"frames-cl-discv5-{p}")]  # UDP only: TCP is beacon REST


def fetch(url):
    return urllib.request.urlopen(url, timeout=10).read()


def find_service():
    root = ET.fromstring(fetch(DESC))
    ns = {"d": "urn:schemas-upnp-org:device-1-0"}
    base = re.match(r"(http://[^/]+)", DESC).group(1)
    for svc in root.iter("{urn:schemas-upnp-org:device-1-0}service"):
        st = svc.findtext("d:serviceType", "", ns)
        if "WANIPConnection" in st or "WANPPPConnection" in st:
            ctrl = svc.findtext("d:controlURL", "", ns)
            if not ctrl.startswith("http"):
                ctrl = base + ("" if ctrl.startswith("/") else "/") + ctrl
            return st, ctrl
    raise SystemExit("no WANIPConnection/WANPPPConnection service found")


def soap(service_type, control_url, action, body=""):
    env = (
        '<?xml version="1.0"?>'
        '<s:Envelope xmlns:s="http://schemas.xmlsoap.org/soap/envelope/" '
        's:encodingStyle="http://schemas.xmlsoap.org/soap/encoding/">'
        f'<s:Body><u:{action} xmlns:u="{service_type}">{body}</u:{action}></s:Body>'
        "</s:Envelope>"
    ).encode()
    req = urllib.request.Request(
        control_url, data=env,
        headers={"Content-Type": 'text/xml; charset="utf-8"',
                 "SOAPAction": f'"{service_type}#{action}"'},
    )
    return urllib.request.urlopen(req, timeout=15).read().decode(errors="ignore")


def main():
    cmd = sys.argv[1] if len(sys.argv) > 1 else "list"
    st, ctrl = find_service()
    print(f"service: {st}\ncontrol: {ctrl}\n")

    if cmd == "add":
        for port, proto, desc in MAPPINGS:
            body = (
                "<NewRemoteHost></NewRemoteHost>"
                f"<NewExternalPort>{port}</NewExternalPort>"
                f"<NewProtocol>{proto}</NewProtocol>"
                f"<NewInternalPort>{port}</NewInternalPort>"
                f"<NewInternalClient>{INTERNAL}</NewInternalClient>"
                "<NewEnabled>1</NewEnabled>"
                f"<NewPortMappingDescription>{desc}</NewPortMappingDescription>"
                "<NewLeaseDuration>0</NewLeaseDuration>"
            )
            try:
                soap(st, ctrl, "AddPortMapping", body)
                print(f"  mapped {proto:3} {port}")
            except Exception as err:
                detail = getattr(err, "read", lambda: b"")()
                code = re.search(rb"<errorCode>(\d+)", detail)
                print(f"  FAILED {proto:3} {port}: {err}"
                      + (f" errorCode={code.group(1).decode()}" if code else ""))
    elif cmd == "delete":
        for port, proto, _ in MAPPINGS:
            body = ("<NewRemoteHost></NewRemoteHost>"
                    f"<NewExternalPort>{port}</NewExternalPort>"
                    f"<NewProtocol>{proto}</NewProtocol>")
            try:
                soap(st, ctrl, "DeletePortMapping", body)
                print(f"  deleted {proto:3} {port}")
            except Exception as err:
                print(f"  not deleted {proto:3} {port}: {err}")

    # Always finish by listing what the gateway actually holds.
    print("\ncurrent mappings on the gateway:")
    for i in range(64):
        try:
            out = soap(st, ctrl, "GetGenericPortMappingEntry",
                       f"<NewPortMappingIndex>{i}</NewPortMappingIndex>")
        except Exception:
            break
        g = lambda t: (re.search(rf"<{t}>([^<]*)</{t}>", out) or [None, "?"])[1]
        print(f"  {g('NewProtocol'):3} {g('NewExternalPort'):>6} -> "
              f"{g('NewInternalClient')}:{g('NewInternalPort'):<6} {g('NewPortMappingDescription')}")


main()
