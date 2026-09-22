#!/usr/bin/env python3
"""RustMail vs Mailpit vs MailHog runtime benchmark (Python stdlib only).

Run `python3 bench.py --help`. Results are appended as JSON lines to
<output-dir>/<tool>.jsonl (default: results/<tool>.jsonl next to this file).
"""

import argparse
import base64
import http.client
import json
import multiprocessing as mp
import os
import random
import shutil
import signal
import socket
import statistics
import subprocess
import sys
import tempfile
import threading
import time
from pathlib import Path

HERE = Path(__file__).resolve().parent
REPO_ROOT = HERE.parent.parent
DEFAULT_RESULTS_DIR = HERE / "results"
DEFAULT_BINARY = REPO_ROOT / "target" / "release" / "rustmail"
DEFAULT_LINUX_BINARY = REPO_ROOT / "target-linux" / "release" / "rustmail"

DEFAULT_PORTS = {
    "rustmail": (9025, 9026),
    "rustmail-docker": (9025, 9026),
    "rustmail-docker-glibc": (9025, 9026),
    "mailpit": (9125, 9126),
    "mailhog": (9225, 9226),
}

RARE_TERM = "quokkazeta"
RARE_EVERY = 10_000
COMMON_TERM = "invoice"
SMALL_TEXT_FILLER = 7
SMALL_HTML_FILLER = 5
MEDIUM_ATTACHMENT_BYTES = 150_000
MEDIUM_DISTINCT_BLOBS = 16
SAMPLE_INTERVAL_S = 1.0
POLL_INTERVAL_S = 0.25
STALL_TIMEOUT_S = 60.0
READY_TIMEOUT_S = 120.0
SOCKET_TIMEOUT_S = 60.0
LATENCY_REQUESTS = 200
LATENCY_WARMUP = 5
DEEP_OFFSET = 90_000
GET_ID_POOL = 50

FILLER = (
    "Thank you for your order. Your invoice is attached below and a copy has been "
    "stored in your account dashboard for future reference. "
)


def small_message(i: int) -> bytes:
    rare = f" {RARE_TERM}" if i % RARE_EVERY == 0 else ""
    text = f"Hello customer {i},\r\n\r\n" + (FILLER + "\r\n") * SMALL_TEXT_FILLER + f"Ref {i}{rare}\r\n"
    html = (
        f"<html><body><p>Hello customer {i},</p>"
        + f"<p>{FILLER}</p>" * SMALL_HTML_FILLER
        + f"<p>Ref {i}{rare}</p></body></html>\r\n"
    )
    return (
        f"From: shop@bench.test\r\nTo: user{i % 997}@bench.test\r\n"
        f"Subject: Order {i} {COMMON_TERM} confirmation\r\n"
        f"Message-ID: <bench-{i}@bench.test>\r\nDate: {time.strftime('%a, %d %b %Y %H:%M:%S +0000', time.gmtime())}\r\n"
        "MIME-Version: 1.0\r\nContent-Type: multipart/alternative; boundary=\"ALT\"\r\n\r\n"
        "--ALT\r\nContent-Type: text/plain; charset=utf-8\r\n\r\n"
        f"{text}"
        "--ALT\r\nContent-Type: text/html; charset=utf-8\r\n\r\n"
        f"{html}"
        "--ALT--\r\n"
    ).encode("utf-8")


def medium_blobs() -> list[str]:
    rng = random.Random(42)
    blobs = []
    for _ in range(MEDIUM_DISTINCT_BLOBS):
        encoded = base64.b64encode(rng.randbytes(MEDIUM_ATTACHMENT_BYTES)).decode("ascii")
        blobs.append("\r\n".join(encoded[k : k + 76] for k in range(0, len(encoded), 76)))
    return blobs


def medium_message(i: int, blobs: list[str]) -> bytes:
    return (
        f"From: reports@bench.test\r\nTo: user{i % 997}@bench.test\r\n"
        f"Subject: Report {i} {COMMON_TERM}\r\n"
        f"Message-ID: <bench-m-{i}@bench.test>\r\nDate: {time.strftime('%a, %d %b %Y %H:%M:%S +0000', time.gmtime())}\r\n"
        "MIME-Version: 1.0\r\nContent-Type: multipart/mixed; boundary=\"MIX\"\r\n\r\n"
        "--MIX\r\nContent-Type: text/plain; charset=utf-8\r\n\r\n"
        f"Report {i} attached.\r\n"
        "--MIX\r\nContent-Type: application/octet-stream\r\n"
        f"Content-Disposition: attachment; filename=\"report-{i}.bin\"\r\n"
        "Content-Transfer-Encoding: base64\r\n\r\n"
        f"{blobs[i % len(blobs)]}\r\n"
        "--MIX--\r\n"
    ).encode("ascii")


def to_data(raw: bytes) -> bytes:
    lines = raw.split(b"\r\n")
    stuffed = [b"." + line if line.startswith(b".") else line for line in lines]
    body = b"\r\n".join(stuffed)
    if not body.endswith(b"\r\n"):
        body += b"\r\n"
    return body + b".\r\n"


def build_payloads(kind: str, count: int) -> list[bytes]:
    if kind == "small":
        return [to_data(small_message(i)) for i in range(count)]
    blobs = medium_blobs()
    return [to_data(medium_message(i, blobs)) for i in range(count)]


class SmtpConn:
    def __init__(self, port: int):
        self.sock = socket.create_connection(("127.0.0.1", port), timeout=SOCKET_TIMEOUT_S)
        self.sock.setsockopt(socket.IPPROTO_TCP, socket.TCP_NODELAY, 1)
        self.buf = b""
        self.reply()
        self.cmd(b"EHLO bench.test\r\n")

    def reply(self) -> int:
        while True:
            while b"\r\n" not in self.buf:
                chunk = self.sock.recv(65536)
                if not chunk:
                    raise ConnectionError("server closed connection")
                self.buf += chunk
            line, self.buf = self.buf.split(b"\r\n", 1)
            if len(line) < 4 or line[3:4] != b"-":
                return int(line[:3])

    def cmd(self, data: bytes) -> int:
        self.sock.sendall(data)
        return self.reply()

    def send(self, index: int, data: bytes) -> int:
        for command in (
            b"MAIL FROM:<shop@bench.test>\r\n",
            f"RCPT TO:<user{index % 997}@bench.test>\r\n".encode("ascii"),
        ):
            code = self.cmd(command)
            if code != 250:
                return code
        code = self.cmd(b"DATA\r\n")
        if code != 354:
            return code
        return self.cmd(data)

    def close(self) -> None:
        try:
            self.cmd(b"QUIT\r\n")
        except OSError:
            pass
        self.sock.close()


PAYLOADS: list[bytes] = []
KEEP_DB = False


def smtp_worker(port: int, worker: int, workers: int, start: mp.Event, out: mp.Queue) -> None:
    accepted = rejected = errors = 0
    codes: dict[int, int] = {}
    conn = SmtpConn(port)
    start.wait()
    for index in range(worker, len(PAYLOADS), workers):
        try:
            code = conn.send(index, PAYLOADS[index])
        except OSError:
            errors += 1
            try:
                conn.sock.close()
                conn = SmtpConn(port)
            except OSError:
                pass
            continue
        if code == 250:
            accepted += 1
        else:
            rejected += 1
            codes[code] = codes.get(code, 0) + 1
            conn.cmd(b"RSET\r\n")
    conn.close()
    out.put({"accepted": accepted, "rejected": rejected, "errors": errors, "codes": codes, "done": time.time()})


class Http:
    def __init__(self, port: int):
        self.port = port
        self.conn = http.client.HTTPConnection("127.0.0.1", port, timeout=SOCKET_TIMEOUT_S)

    def request(self, method: str, path: str) -> tuple[int, bytes]:
        for attempt in range(2):
            try:
                self.conn.request(method, path)
                resp = self.conn.getresponse()
                return resp.status, resp.read()
            except (OSError, http.client.HTTPException):
                self.conn.close()
                self.conn = http.client.HTTPConnection("127.0.0.1", self.port, timeout=SOCKET_TIMEOUT_S)
                if attempt:
                    raise
        raise RuntimeError("unreachable")

    def json(self, method: str, path: str) -> dict:
        status, body = self.request(method, path)
        if status >= 400:
            raise RuntimeError(f"{method} {path} -> HTTP {status}: {body[:200]!r}")
        return json.loads(body) if body.strip() else {}


class Api:
    count_path = ""
    list_path = ""
    search_path = ""
    get_path = ""
    delete_all_path = ""
    total_key = "total"

    def __init__(self, port: int):
        self.http = Http(port)

    def count(self) -> int:
        return int(self.http.json("GET", self.count_path)[self.total_key])

    def list(self, offset: int, limit: int = 50) -> str:
        return self.list_path.format(offset=offset, limit=limit)

    def search(self, term: str) -> str:
        return self.search_path.format(term=term)

    def get(self, msg_id: str) -> str:
        return self.get_path.format(id=msg_id)

    def ids(self, offset: int, limit: int) -> list[str]:
        return [m[self.id_key] for m in self.messages(self.http.json("GET", self.list(offset, limit)))]

    def messages(self, payload: dict) -> list[dict]:
        return payload["messages"]

    id_key = "id"


class RustmailApi(Api):
    count_path = "/api/v1/messages?limit=1"
    list_path = "/api/v1/messages?limit={limit}&offset={offset}"
    search_path = "/api/v1/messages?q={term}&limit=50"
    get_path = "/api/v1/messages/{id}"
    delete_all_path = "/api/v1/messages"


class MailpitApi(Api):
    count_path = "/api/v1/messages?limit=1"
    list_path = "/api/v1/messages?limit={limit}&start={offset}"
    search_path = "/api/v1/search?query={term}&limit=50"
    get_path = "/api/v1/message/{id}"
    delete_all_path = "/api/v1/messages"
    id_key = "ID"


class MailhogApi(Api):
    count_path = "/api/v2/messages?limit=1"
    list_path = "/api/v2/messages?start={offset}&limit={limit}"
    search_path = "/api/v2/search?kind=containing&query={term}&limit=50"
    get_path = "/api/v1/messages/{id}"
    delete_all_path = "/api/v1/messages"
    id_key = "ID"

    def messages(self, payload: dict) -> list[dict]:
        return payload["items"]


def cpu_seconds_from_ps(value: str) -> float:
    parts = value.strip().split(":")
    seconds = float(parts[-1])
    if len(parts) >= 2:
        seconds += 60 * float(parts[-2])
    if len(parts) == 3:
        seconds += 3600 * float(parts[-3])
    return seconds


class NativeRustmail:
    name = "rustmail"
    api_class = RustmailApi

    def __init__(self, binary: Path, smtp_port: int, http_port: int):
        self.binary = binary
        self.smtp_port = smtp_port
        self.http_port = http_port
        self.dir = Path(tempfile.mkdtemp(prefix="rmbench-"))
        self.proc: subprocess.Popen | None = None
        self.log = open(self.dir / "server.log", "ab")

    def start(self) -> None:
        env = {k: v for k, v in os.environ.items() if not k.startswith("RUSTMAIL_")}
        self.proc = subprocess.Popen(
            [
                str(self.binary), "--bind", "127.0.0.1",
                "--smtp-port", str(self.smtp_port), "--http-port", str(self.http_port),
                "--db-path", str(self.dir / "rustmail.db"), "--log-level", "warn",
            ],
            stdout=self.log, stderr=self.log, env=env,
        )

    def stop(self) -> None:
        if self.proc and self.proc.poll() is None:
            self.proc.send_signal(signal.SIGTERM)
            try:
                self.proc.wait(timeout=30)
            except subprocess.TimeoutExpired:
                self.proc.kill()
                self.proc.wait()

    def restart(self) -> None:
        self.stop()
        self.start()

    def destroy(self) -> None:
        self.stop()
        self.log.close()
        if not KEEP_DB:
            shutil.rmtree(self.dir, ignore_errors=True)

    def sample(self) -> tuple[float, float]:
        out = subprocess.run(
            ["ps", "-o", "rss=,time=", "-p", str(self.proc.pid)], capture_output=True, text=True
        ).stdout.split()
        return int(out[0]) * 1024, cpu_seconds_from_ps(out[1])

    def footprint(self) -> str:
        out = subprocess.run(["footprint", "-p", str(self.proc.pid)], capture_output=True, text=True).stdout
        return next((line.strip() for line in out.splitlines() if "Footprint" in line), "")

    def disk_bytes(self) -> int:
        return sum(p.stat().st_size for p in self.dir.glob("rustmail.db*"))


class DockerTool:
    image = ""
    args: list[str] = []
    container_smtp = 1025
    container_http = 8025
    data_dir = "/data"

    def __init__(self, name: str, smtp_port: int, http_port: int, api_class: type[Api]):
        self.name = name
        self.smtp_port = smtp_port
        self.http_port = http_port
        self.api_class = api_class
        self.container = f"bench-{name}"

    def docker(self, *args: str) -> str:
        return subprocess.run(["docker", *args], capture_output=True, text=True, check=True).stdout

    def start(self) -> None:
        self.destroy()
        self.docker(
            "run", "-d", "--name", self.container,
            "-p", f"127.0.0.1:{self.smtp_port}:{self.container_smtp}",
            "-p", f"127.0.0.1:{self.http_port}:{self.container_http}",
            "-v", f"{self.container}-data:{self.data_dir}",
            *self.extra_run_args(), self.image, *self.args,
        )

    def extra_run_args(self) -> list[str]:
        return []

    def restart(self) -> None:
        self.docker("restart", "-t", "30", self.container)

    def stop(self) -> None:
        subprocess.run(["docker", "stop", "-t", "30", self.container], capture_output=True)

    def destroy(self) -> None:
        subprocess.run(["docker", "rm", "-f", "-v", self.container], capture_output=True)
        subprocess.run(["docker", "volume", "rm", "-f", f"{self.container}-data"], capture_output=True)

    def sample(self) -> tuple[float, float]:
        out = self.docker(
            "exec", self.container, "sh", "-c",
            "grep '^anon ' /sys/fs/cgroup/memory.stat; grep '^usage_usec' /sys/fs/cgroup/cpu.stat",
        ).split()
        return int(out[1]), int(out[3]) / 1e6

    def footprint(self) -> str:
        return ""

    def disk_bytes(self) -> int | None:
        out = self.docker("exec", self.container, "du", "-sk", self.data_dir)
        return int(out.split()[0]) * 1024


class MailpitDocker(DockerTool):
    image = "axllent/mailpit:latest"
    args = ["--database", "/data/mailpit.db", "--max", "0"]

    def __init__(self, smtp_port: int, http_port: int):
        super().__init__("mailpit", smtp_port, http_port, MailpitApi)


class MailhogDocker(DockerTool):
    image = "mailhog/mailhog:latest"
    container_http = 8025
    args: list[str] = []

    persistent = False

    def __init__(self, smtp_port: int, http_port: int):
        super().__init__("mailhog", smtp_port, http_port, MailhogApi)

    def extra_run_args(self) -> list[str]:
        return ["--platform", "linux/amd64"]

    def disk_bytes(self) -> int | None:
        return None


class RustmailDocker(DockerTool):
    image = "ghcr.io/rustmailapp/rustmail:latest"
    args = ["--log-level", "warn"]

    def __init__(self, smtp_port: int, http_port: int):
        super().__init__("rustmail-docker", smtp_port, http_port, RustmailApi)


class RustmailDockerGlibc(DockerTool):
    image = "debian:bookworm-slim"

    def __init__(self, smtp_port: int, http_port: int, linux_binary: Path):
        super().__init__("rustmail-docker-glibc", smtp_port, http_port, RustmailApi)
        self.linux_binary = linux_binary
        self.args = [
            "/opt/rustmail", "--bind", "0.0.0.0", "--smtp-port", str(self.container_smtp),
            "--http-port", str(self.container_http), "--db-path", "/data/rustmail.db", "--log-level", "warn",
        ]

    def extra_run_args(self) -> list[str]:
        return ["-v", f"{self.linux_binary}:/opt/rustmail:ro", "--entrypoint", "/usr/bin/env"]


class Sampler(threading.Thread):
    def __init__(self, tool):
        super().__init__(daemon=True)
        self.tool = tool
        self.stop_event = threading.Event()
        self.samples: list[tuple[float, float, float]] = []

    def run(self) -> None:
        while not self.stop_event.is_set():
            try:
                rss, cpu = self.tool.sample()
                self.samples.append((time.time(), rss, cpu))
            except (subprocess.CalledProcessError, IndexError, ValueError):
                pass
            self.stop_event.wait(SAMPLE_INTERVAL_S)

    def finish(self) -> dict:
        self.stop_event.set()
        self.join()
        if len(self.samples) < 2:
            return {}
        (t0, _, c0), (t1, _, c1) = self.samples[0], self.samples[-1]
        return {
            "peak_rss_mb": round(max(s[1] for s in self.samples) / 2**20, 1),
            "end_rss_mb": round(self.samples[-1][1] / 2**20, 1),
            "cpu_pct_avg": round(100 * (c1 - c0) / (t1 - t0), 1),
            "cpu_seconds": round(c1 - c0, 2),
        }


def wait_ready(api: Api, expected: int | None = None) -> float:
    started = time.time()
    while time.time() - started < READY_TIMEOUT_S:
        try:
            count = api.count()
            if expected is None or count >= expected:
                return time.time() - started
        except (OSError, RuntimeError, http.client.HTTPException, ValueError, KeyError):
            pass
        time.sleep(0.05)
    raise TimeoutError(f"API on port {api.http.port} not ready (expected={expected}) after {READY_TIMEOUT_S}s")


def ingest(tool, api: Api, conns: int, expected_total: int) -> dict:
    ctx = mp.get_context("fork")
    start, out = ctx.Event(), ctx.Queue()
    procs = [ctx.Process(target=smtp_worker, args=(tool.smtp_port, w, conns, start, out)) for w in range(conns)]
    for p in procs:
        p.start()
    time.sleep(1.0)
    sampler = Sampler(tool)
    sampler.start()
    t0 = time.time()
    start.set()
    reports = [out.get() for _ in procs]
    for p in procs:
        p.join()
    t_smtp = max(r["done"] for r in reports)
    last_count, last_change = -1, time.time()
    t_stored = None
    while True:
        count = api.count()
        now = time.time()
        if count != last_count:
            last_count, last_change = count, now
        if count >= expected_total:
            t_stored = now
            break
        if now - last_change > STALL_TIMEOUT_S:
            break
        time.sleep(POLL_INTERVAL_S)
    stats = sampler.finish()
    sent = len(PAYLOADS)
    accepted = sum(r["accepted"] for r in reports)
    codes: dict[str, int] = {}
    for r in reports:
        for code, n in r["codes"].items():
            codes[str(code)] = codes.get(str(code), 0) + n
    end = t_stored or time.time()
    return {
        "sent": sent,
        "accepted": accepted,
        "rejected": sum(r["rejected"] for r in reports),
        "conn_errors": sum(r["errors"] for r in reports),
        "reject_codes": codes,
        "stored": last_count,
        "smtp_seconds": round(t_smtp - t0, 2),
        "accepted_per_s": round(accepted / (t_smtp - t0), 1),
        "stored_seconds": round(end - t0, 2),
        "stored_per_s": round(last_count / (end - t0), 1) if last_count > 0 else 0,
        "drain_after_smtp_s": round(max(0.0, end - t_smtp), 2),
        "complete": t_stored is not None,
        **stats,
    }


def percentiles(samples: list[float]) -> dict:
    ordered = sorted(samples)
    pick = lambda q: ordered[min(len(ordered) - 1, int(q * len(ordered)))]
    return {
        "p50_ms": round(pick(0.50) * 1000, 2),
        "p95_ms": round(pick(0.95) * 1000, 2),
        "p99_ms": round(pick(0.99) * 1000, 2),
        "mean_ms": round(statistics.fmean(ordered) * 1000, 2),
    }


def time_requests(api: Api, paths: list[str]) -> dict:
    for path in paths[:LATENCY_WARMUP]:
        api.http.request("GET", path)
    durations, errors = [], 0
    for i in range(LATENCY_REQUESTS):
        path = paths[i % len(paths)]
        began = time.perf_counter()
        status, _ = api.http.request("GET", path)
        durations.append(time.perf_counter() - began)
        errors += status >= 400
    return {**percentiles(durations), "errors": errors, "n": LATENCY_REQUESTS}


def http_scenario(tool, api: Api, stored: int) -> dict:
    deep = min(DEEP_OFFSET, max(0, stored - 50))
    ids = []
    for offset in range(0, stored, max(1, stored // GET_ID_POOL)):
        ids.extend(api.ids(offset, 1))
    results = {
        "list_first": time_requests(api, [api.list(0)]),
        "list_deep": time_requests(api, [api.list(deep)]),
        "search_rare": time_requests(api, [api.search(RARE_TERM)]),
        "search_common": time_requests(api, [api.search(COMMON_TERM)]),
        "get_single": time_requests(api, [api.get(i) for i in ids]),
    }
    rare_hits = api.http.json("GET", api.search(RARE_TERM))
    common_hits = api.http.json("GET", api.search(COMMON_TERM))
    results["search_rare_total"] = rare_hits.get("messages_count", rare_hits.get("total"))
    results["search_common_total"] = common_hits.get("messages_count", common_hits.get("total"))
    results["list_deep_offset"] = deep
    return results


def restart_scenario(tool, api: Api, stored: int) -> dict:
    before_rss, _ = tool.sample()
    footprint_before = tool.footprint()
    began = time.time()
    tool.restart()
    ready = wait_ready(api)
    first_count = api.count()
    full_ready = time.time() - began
    if first_count < stored and getattr(tool, "persistent", True):
        full_ready = wait_ready(api, stored) + ready
    total = time.time() - began
    time.sleep(10)
    idle_rss, _ = tool.sample()
    return {
        "rss_before_restart_mb": round(before_rss / 2**20, 1),
        "footprint_before_restart": footprint_before,
        "restart_to_api_s": round(full_ready, 2),
        "restart_total_s": round(total, 2),
        "count_after_restart": first_count,
        "idle_rss_after_restart_mb": round(idle_rss / 2**20, 1),
        "footprint_idle_after_restart": tool.footprint(),
    }


def delete_all(tool, api: Api) -> dict:
    began = time.perf_counter()
    status, body = api.http.request("DELETE", api.delete_all_path)
    elapsed = time.perf_counter() - began
    remaining = api.count()
    return {"delete_all_s": round(elapsed, 3), "status": status, "remaining": remaining, "body": body[:120].decode("utf-8", "replace")}


def make_tool(args: argparse.Namespace) -> NativeRustmail | DockerTool:
    default_smtp, default_http = DEFAULT_PORTS[args.tool]
    smtp_port = args.smtp_port or default_smtp
    http_port = args.http_port or default_http
    if args.tool == "rustmail":
        return NativeRustmail(Path(args.binary), smtp_port, http_port)
    if args.tool == "mailpit":
        return MailpitDocker(smtp_port, http_port)
    if args.tool == "mailhog":
        return MailhogDocker(smtp_port, http_port)
    if args.tool == "rustmail-docker":
        return RustmailDocker(smtp_port, http_port)
    if args.tool == "rustmail-docker-glibc":
        return RustmailDockerGlibc(smtp_port, http_port, Path(args.linux_binary))
    raise ValueError(f"unknown tool {args.tool!r}")


def record(results_dir: Path, tool_name: str, entry: dict) -> None:
    results_dir.mkdir(parents=True, exist_ok=True)
    with open(results_dir / f"{tool_name}.jsonl", "a", encoding="utf-8") as fh:
        fh.write(json.dumps(entry) + "\n")
    print(json.dumps(entry), flush=True)


def run(args: argparse.Namespace) -> None:
    global PAYLOADS, KEEP_DB
    KEEP_DB = args.keep_db
    built = time.time()
    PAYLOADS = build_payloads(args.kind, args.count)
    print(f"built {len(PAYLOADS)} {args.kind} payloads, avg {sum(map(len, PAYLOADS)) // len(PAYLOADS)} B, in {time.time() - built:.1f}s", flush=True)
    tool = make_tool(args)
    try:
        tool.start()
        api = tool.api_class(tool.http_port)
        startup = wait_ready(api)
        idle_rss, _ = tool.sample()
        entry = {
            "tool": args.tool, "scenario": args.scenario, "kind": args.kind, "count": args.count,
            "conns": args.conns, "startup_s": round(startup, 2), "idle_rss_empty_mb": round(idle_rss / 2**20, 1),
            "ts": time.strftime("%Y-%m-%dT%H:%M:%S%z"),
        }
        entry["ingest"] = ingest(tool, api, args.conns, args.count)
        time.sleep(2)
        entry["disk_mb"] = None if (d := tool.disk_bytes()) is None else round(d / 2**20, 1)
        stored = entry["ingest"]["stored"]
        if args.http:
            entry["http"] = http_scenario(tool, api, stored)
        if args.restart:
            entry["restart"] = restart_scenario(tool, api, stored)
        if args.http:
            entry["delete"] = delete_all(tool, api)
            time.sleep(2)
            entry["disk_mb_after_delete"] = None if (d := tool.disk_bytes()) is None else round(d / 2**20, 1)
        record(Path(args.output_dir), args.tool, entry)
    finally:
        tool.destroy()


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--tool", required=True, choices=["rustmail", "mailpit", "mailhog", "rustmail-docker", "rustmail-docker-glibc"])
    parser.add_argument("--scenario", required=True)
    parser.add_argument("--kind", choices=["small", "medium"], default="small")
    parser.add_argument("--count", type=int, default=100_000)
    parser.add_argument("--conns", type=int, default=8)
    parser.add_argument("--http", action="store_true", help="run S3 HTTP latency + delete-all after ingest")
    parser.add_argument("--restart", action="store_true", help="run S4 restart + idle RSS after ingest")
    parser.add_argument("--binary", default=str(DEFAULT_BINARY), help="rustmail (native) binary path")
    parser.add_argument("--linux-binary", default=str(DEFAULT_LINUX_BINARY), help="linux glibc rustmail binary, for --tool rustmail-docker-glibc")
    parser.add_argument("--smtp-port", type=int, default=None, help="override the tool's default SMTP port")
    parser.add_argument("--http-port", type=int, default=None, help="override the tool's default HTTP port")
    parser.add_argument("--output-dir", default=str(DEFAULT_RESULTS_DIR), help="where <tool>.jsonl result files are appended")
    parser.add_argument("--keep-db", action="store_true", help="native rustmail: keep the temp DB dir for inspection")
    run(parser.parse_args())


if __name__ == "__main__":
    sys.exit(main())
