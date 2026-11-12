#!/usr/bin/env python3
"""Serve one captured snapshot to Titan's UNMODIFIED shared quote assertions.

Usage: python3 tests/support/fixture_rpc.py tests/fixtures/<capture>.bin.gz [port]
Prints a localhost URL. Only getVersion/getAccountInfo/getMultipleAccounts exist;
there is no upstream RPC, transaction submission, or mutation endpoint.
"""
import base64
import gzip
import json
import struct
import sys
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer


def base58(data):
    alphabet = "123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz"
    n, out = int.from_bytes(data, "big"), ""
    while n:
        n, r = divmod(n, 58)
        out = alphabet[r] + out
    return "1" * (len(data) - len(data.lstrip(b"\0"))) + out


def snapshot(path):
    data = gzip.decompress(open(path, "rb").read())
    offset = 0

    def integer():
        nonlocal offset
        value = struct.unpack_from("<Q", data, offset)[0]
        offset += 8
        return value

    def vector():
        nonlocal offset
        size = integer()
        value = data[offset:offset + size]
        offset += size
        return value

    accounts, slot = {}, None
    for _ in range(integer()):
        key, encoded = vector().decode(), vector()
        if encoded == b"\0":
            accounts[key] = None
            continue
        assert encoded[0] == 1
        lamports, size = struct.unpack_from("<QQ", encoded, 1)
        payload = encoded[17:17 + size]
        tail = 17 + size
        assert len(encoded) == tail + 41
        accounts[key] = {
            "lamports": lamports,
            "data": [base64.b64encode(payload).decode(), "base64"],
            "owner": base58(encoded[tail:tail + 32]),
            "executable": bool(encoded[tail + 32]),
            "rentEpoch": struct.unpack_from("<Q", encoded, tail + 33)[0],
            "space": size,
        }
        if key == "SysvarC1ock11111111111111111111111111111111":
            slot = struct.unpack_from("<Q", payload)[0]
    assert offset == len(data) and slot is not None
    return accounts, slot


if __name__ == "__main__":
    accounts, slot = snapshot(sys.argv[1])

    class Handler(BaseHTTPRequestHandler):
        def log_message(self, *_):
            pass

        def do_POST(self):
            query = json.loads(self.rfile.read(int(self.headers["Content-Length"])))
            method, params = query["method"], query.get("params", [])
            response = {"jsonrpc": "2.0", "id": query["id"]}
            if method == "getVersion":
                response["result"] = {"solana-core": "2.3.0", "feature-set": 0}
            elif method in ("getAccountInfo", "getMultipleAccounts"):
                keys = [params[0]] if method == "getAccountInfo" else params[0]
                missing = [key for key in keys if key not in accounts]
                if missing:
                    response["error"] = {"code": -32602, "message": "fixture missing " + str(missing)}
                else:
                    values = [accounts[key] for key in keys]
                    response["result"] = {"context": {"slot": slot}, "value": values[0] if method == "getAccountInfo" else values}
            else:
                response["error"] = {"code": -32601, "message": "read-only fixture RPC: unsupported method"}
            body = json.dumps(response).encode()
            self.send_response(200)
            self.send_header("Content-Type", "application/json")
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)

    server = ThreadingHTTPServer(("127.0.0.1", int(sys.argv[2]) if len(sys.argv) > 2 else 0), Handler)
    print(f"http://127.0.0.1:{server.server_port}", flush=True)
    server.serve_forever()
