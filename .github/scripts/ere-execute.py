#!/usr/bin/env python3
"""Execute an ELF on a running `ere-server` and write its public values.

`ere-server` has no `execute` subcommand — execution is a Twirp RPC
(`/twirp/api.ZkvmService/Execute`) served while the process runs in server mode.
See `crates/server/api/proto/api.proto` in eth-act/ere:

    message ExecuteRequest  { bytes input_stdin = 1; optional bytes input_proofs = 2; }
    message ExecuteResponse { oneof result { ExecuteOk ok = 1; string err = 2; } }
    message ExecuteOk       { bytes public_values = 1; bytes report = 2; }

Speaks protobuf rather than Twirp's JSON because the generated types carry a bare
`#[derive(serde::Serialize)]`, so JSON would encode `bytes` as an array of
integers — enormous for a multi-megabyte witness. Every field either side uses is
length-delimited, which is the only wire type handled below.

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
    """Yield (field_number, payload) for each length-delimited field, skipping
    varint fields so an added scalar does not break parsing."""
    pos = 0
    while pos < len(buf):
        key, pos = decode_varint(buf, pos)
        wire = key & 0x07
        if wire == 2:
            length, pos = decode_varint(buf, pos)
            yield key >> 3, buf[pos : pos + length]
            pos += length
        elif wire == 0:
            _, pos = decode_varint(buf, pos)
        else:
            raise ValueError(f"unsupported wire type {wire} for field {key >> 3}")


def decode_execute_response(body: bytes) -> bytes:
    """Return `ok.public_values`, or raise with the server's `err` string."""
    for field, payload in fields(body):
        if field == 1:  # ExecuteOk
            for inner_field, inner in fields(payload):
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

    # ExecuteRequest with only `input_stdin` (field 1, length-delimited).
    request = urllib.request.Request(
        url,
        data=b"\x0a" + encode_varint(len(stdin)) + stdin,
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
