#!/usr/bin/env python3
"""Chromium's own memory breakdown of the game's page.

    python3 parity/perf/cdp_memory.py PORT

PORT is the page's DevTools port: `bb --devtools PORT`, or `memory_inventory --devtools
PORT --hold SECONDS` (which keeps the session running after its last point). Over the
DevTools protocol (the browser target) it traces one detailed memory-infra dump and prints,
per process, the top-level allocators' sizes (malloc, partition_alloc, blink_gc, v8, cc,
discardable, shared_memory, web_cache, ...) and the largest of each's parts. The standard
library only: a minimal WebSocket client.
"""
import base64, json, os, socket, struct, sys, urllib.request
from collections import defaultdict

def http_json(port, path):
    req = urllib.request.Request(f"http://127.0.0.1:{port}{path}")
    with urllib.request.urlopen(req, timeout=10) as r:
        return json.loads(r.read())


class WS:
    def __init__(self, url):
        rest = url.split("://", 1)[1]
        hostport, path = rest.split("/", 1)
        host, port = hostport.split(":")
        origin = f"http://127.0.0.1:{port}"
        self.s = socket.create_connection((host, int(port)), timeout=60)
        key = base64.b64encode(os.urandom(16)).decode()
        self.s.sendall((f"GET /{path} HTTP/1.1\r\nHost: {hostport}\r\nUpgrade: websocket\r\nConnection: Upgrade\r\n"
                        f"Sec-WebSocket-Key: {key}\r\nSec-WebSocket-Version: 13\r\nOrigin: {origin}\r\n\r\n").encode())
        head = b""
        while b"\r\n\r\n" not in head:
            head += self.s.recv(1)
        if b" 101 " not in head.split(b"\r\n")[0]:
            raise RuntimeError(head.decode(errors="replace"))
        self.buf = b""
        self.next_id = 0

    def _read(self, n):
        while len(self.buf) < n:
            chunk = self.s.recv(1 << 20)
            if not chunk:
                raise EOFError
            self.buf += chunk
        out, self.buf = self.buf[:n], self.buf[n:]
        return out

    def send(self, method, params=None):
        self.next_id += 1
        data = json.dumps({"id": self.next_id, "method": method, "params": params or {}}).encode()
        mask = os.urandom(4)
        n = len(data)
        head = bytes([0x81]) + (bytes([0x80 | n]) if n < 126 else bytes([0x80 | 126]) + struct.pack(">H", n) if n < 65536 else bytes([0x80 | 127]) + struct.pack(">Q", n))
        self.s.sendall(head + mask + bytes(b ^ mask[i % 4] for i, b in enumerate(data)))
        return self.next_id

    def recv(self):
        message = b""
        while True:
            b0, b1 = self._read(2)
            n = b1 & 0x7F
            if n == 126:
                n = struct.unpack(">H", self._read(2))[0]
            elif n == 127:
                n = struct.unpack(">Q", self._read(8))[0]
            payload = self._read(n)
            if b0 & 0x0F == 0x9:  # ping
                continue
            message += payload
            if b0 & 0x80:
                return json.loads(message)

    def call(self, method, params=None, events=None):
        i = self.send(method, params)
        while True:
            m = self.recv()
            if m.get("id") == i:
                if "error" in m:
                    raise RuntimeError(f"{method}: {m['error']}")
                return m.get("result")
            if events is not None:
                events.append(m)


def size_of(attrs):
    a = attrs.get("size")
    if not a:
        return 0
    v = a["value"]
    return int(v, 16) if isinstance(v, str) else int(v)


def main():
    version = http_json(int(sys.argv[1]), "/json/version")
    ws = WS(version["webSocketDebuggerUrl"])
    events = []
    ws.call("Tracing.start", {
        "traceConfig": {"includedCategories": ["disabled-by-default-memory-infra"], "memoryDumpConfig": {"triggers": []}},
        "transferMode": "ReportEvents",
    }, events)
    ws.call("Tracing.requestMemoryDump", {"deterministic": True, "levelOfDetail": "detailed"}, events)
    ws.send("Tracing.end")
    trace = []
    while True:
        m = ws.recv()
        if m.get("method") == "Tracing.dataCollected":
            trace += m["params"]["value"]
        elif m.get("method") == "Tracing.tracingComplete":
            break
    names = {}
    for e in trace:
        if e.get("ph") == "M" and e.get("name") == "process_name":
            names[e["pid"]] = e["args"]["name"]
    per_pid = defaultdict(dict)
    for e in trace:
        if e.get("ph") == "v" and "dumps" in e.get("args", {}):
            for name, d in e["args"]["dumps"].get("allocators", {}).items():
                per_pid[e["pid"]][name] = size_of(d.get("attrs", {}))
    MB = 1 << 20
    for pid, allocs in per_pid.items():
        top = {k: v for k, v in allocs.items() if "/" not in k}
        print(f"pid {pid} ({names.get(pid, '?')}):")
        for k, v in sorted(top.items(), key=lambda x: -x[1]):
            if v >= MB // 2:
                subs = sorted(((s, sv) for s, sv in allocs.items() if s.startswith(k + "/") and s.count("/") == 1), key=lambda x: -x[1])[:5]
                detail = ", ".join(f"{s.split('/', 1)[1]} {sv / MB:.1f}" for s, sv in subs if sv >= MB // 2)
                print(f"   {v / MB:8.1f} MB  {k}" + (f"   ({detail})" if detail else ""))


if __name__ == "__main__":
    main()
