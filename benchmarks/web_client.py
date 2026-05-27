#!/usr/bin/env python3
import argparse
import socket
import sys


def request(port: int, path: str) -> bytes:
    payload = (
        f"GET {path} HTTP/1.1\r\n"
        f"Host: 127.0.0.1:{port}\r\n"
        "Connection: close\r\n"
        "\r\n"
    ).encode("ascii")
    with socket.create_connection(("127.0.0.1", port), timeout=2.0) as sock:
        sock.sendall(payload)
        chunks = []
        while True:
            chunk = sock.recv(65536)
            if not chunk:
                break
            chunks.append(chunk)
    return b"".join(chunks)


def body(response: bytes) -> bytes:
    marker = b"\r\n\r\n"
    offset = response.find(marker)
    if offset < 0:
        return b""
    return response[offset + len(marker):]


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--port", type=int, required=True)
    parser.add_argument("--requests", type=int, default=1000)
    args = parser.parse_args()
    expected = b'{"ok":true,"service":"ax"}'
    score = 0
    for index in range(args.requests):
        path = "/health" if index % 2 == 0 else "/ping"
        response = request(args.port, path)
        if path == "/health":
            if body(response) != expected:
                return 2
            score += len(expected)
        else:
            if body(response) != b"pong":
                return 3
            score += 4
    return 0 if score > 0 else 1


if __name__ == "__main__":
    sys.exit(main())
