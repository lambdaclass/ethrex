#!/usr/bin/env python3
"""Execute an ELF on a running `ere-server` and write its public values.

`ere-server` has no `execute` subcommand — execution is a Twirp RPC
(`/twirp/api.ZkvmService/Execute`) served while the process runs in server mode.
See `crates/server/api/proto/api.proto` in eth-act/ere:

    message ExecuteRequest  { bytes input_stdin = 1; optional bytes input_proofs = 2; }
    message ExecuteResponse { oneof result { ExecuteOk ok = 1; string err = 2; } }
    message ExecuteOk       { bytes public_values = 1; bytes report = 2; }

Speaks protobuf rather than Twirp's JSON on purpose. The generated types carry a
bare `#[derive(serde::Serialize)]` with no `rename_all` and no base64 helper, so
JSON would encode `bytes` as an array of integers — for a multi-megabyte witness
that is both enormous and needless. The two messages here are small enough to
encode and decode by hand, which also removes any guesswork about field naming.

Usage: ere-execute.py <url> <input-file> <output-file>
"""

import sys
import urllib.error
import urllib.request


def encode_varint(value: int) -> bytes:
    out = bytearray()
    while True:
        byte = value & 0x7F
        value >>= 7
        out.append(byte | (0x80 if value else 0))
        if not value:
            return bytes(out)


def decode_varint(buf: bytes, pos: int) -> tuple[int, int]:
    value = shift = 0
    while True:
        if pos >= len(buf):
            raise ValueError("truncated varint")
        byte = buf[pos]
        pos += 1
        value |= (byte & 0x7F) << shift
        if not byte & 0x80:
            return value, pos
        shift += 7
        if shift > 63:
            raise ValueError("varint too long")


def fields(buf: bytes):
    """Yield (field_number, wire_type, payload) for a protobuf message."""
    pos = 0
    while pos < len(buf):
        key, pos = decode_varint(buf, pos)
        field, wire = key >> 3, key & 0x07
        if wire == 2:  # length-delimited
            length, pos = decode_varint(buf, pos)
            yield field, wire, buf[pos : pos + length]
            pos += length
        elif wire == 0:  # varint
            value, pos = decode_varint(buf, pos)
            yield field, wire, value
        elif wire == 5:
            yield field, wire, buf[pos : pos + 4]
            pos += 4
        elif wire == 1:
            yield field, wire, buf[pos : pos + 8]
            pos += 8
        else:
            raise ValueError(f"unsupported wire type {wire} for field {field}")


def encode_execute_request(stdin: bytes) -> bytes:
    """ExecuteRequest with only `input_stdin` (field 1, length-delimited)."""
    return b"\x0a" + encode_varint(len(stdin)) + stdin


def decode_execute_response(body: bytes) -> bytes:
    """Return `ok.public_values`, or raise with the server's `err` string."""
    for field, _wire, payload in fields(body):
        if field == 1:  # ExecuteOk
            for inner_field, _w, inner in fields(payload):
                if inner_field == 1:  # public_values
                    return inner
            raise ValueError("ExecuteOk carried no public_values")
        if field == 2:  # err
            raise RuntimeError(f"guest execution failed: {payload.decode('utf-8', 'replace')}")
    raise ValueError("ExecuteResponse set neither ok nor err")


def main() -> int:
    if len(sys.argv) != 4:
        print(__doc__, file=sys.stderr)
        return 2
    url, input_path, output_path = sys.argv[1], sys.argv[2], sys.argv[3]

    with open(input_path, "rb") as handle:
        stdin = handle.read()
    print(f"executing with {len(stdin)} bytes of statelessInputBytes")

    request = urllib.request.Request(
        url,
        data=encode_execute_request(stdin),
        headers={"Content-Type": "application/protobuf"},
        method="POST",
    )
    try:
        with urllib.request.urlopen(request, timeout=1800) as response:
            body = response.read()
    except urllib.error.HTTPError as err:
        detail = err.read().decode("utf-8", "replace")
        print(f"ere-server returned HTTP {err.code}: {detail}", file=sys.stderr)
        return 1

    public_values = decode_execute_response(body)
    with open(output_path, "wb") as handle:
        handle.write(public_values)
    print(f"wrote {len(public_values)} bytes of statelessOutputBytes to {output_path}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
