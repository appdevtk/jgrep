#!/usr/bin/env python3
"""Optional real k9s + kubectl PTY test against a localhost-only synthetic API.

No cluster credentials, host k9s configuration or real Kubernetes API is used.
Usage: python3 scripts/test-k9s.py /path/to/jgrep
"""
from __future__ import annotations

import importlib.util
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import threading
import unittest
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from urllib.parse import parse_qs, urlsplit

spec = importlib.util.spec_from_file_location("tui", Path(__file__).with_name("test-tui.py"))
assert spec is not None and spec.loader is not None
tui = importlib.util.module_from_spec(spec)
spec.loader.exec_module(tui)

NAMESPACE = "synthetic"
POD = {
    "apiVersion": "v1", "kind": "Pod",
    "metadata": {"name": "stream-test", "namespace": NAMESPACE, "uid": "synthetic-pod", "resourceVersion": "1"},
    "spec": {"containers": [{"name": "app", "image": "synthetic:test"}]},
    "status": {"phase": "Running", "containerStatuses": [{
        "name": "app", "ready": True, "restartCount": 0, "image": "synthetic:test",
        "imageID": "synthetic", "state": {"running": {}},
    }]},
}


class SyntheticAPI(ThreadingHTTPServer):
    daemon_threads = True

    def __init__(self) -> None:
        super().__init__(("127.0.0.1", 0), Handler)
        self.stop = threading.Event()
        self.first = threading.Event()
        self.second = threading.Event()
        self.logs_open = threading.Event()
        self.requests: list[tuple[str, str]] = []


class Handler(BaseHTTPRequestHandler):
    server: SyntheticAPI

    def log_message(self, format: str, *args: object) -> None:
        pass

    def respond(self, value: object) -> None:
        data = json.dumps(value).encode()
        self.send_response(200)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(data)))
        self.end_headers()
        self.wfile.write(data)

    def do_POST(self) -> None:
        self.server.requests.append(("POST", self.path))
        self.rfile.read(int(self.headers.get("Content-Length", "0")))
        path = urlsplit(self.path).path
        if path.endswith("/selfsubjectaccessreviews"):
            self.respond({"apiVersion": "authorization.k8s.io/v1", "kind": "SelfSubjectAccessReview", "status": {"allowed": True}})
        elif path.endswith("/selfsubjectrulesreviews"):
            self.respond({"apiVersion": "authorization.k8s.io/v1", "kind": "SelfSubjectRulesReview", "status": {"resourceRules": [{"verbs": ["get", "list", "watch"], "apiGroups": ["*"], "resources": ["*"]}], "incomplete": False}})
        else:
            self.send_error(405)

    def do_GET(self) -> None:
        self.server.requests.append(("GET", self.path))
        url = urlsplit(self.path)
        path, query = url.path, parse_qs(url.query)
        try:
            if path == "/version":
                self.respond({"major": "1", "minor": "32", "gitVersion": "v1.32.0", "platform": "synthetic"})
            elif path in ("/healthz", "/readyz", "/livez"):
                self.respond("ok")
            elif path == "/api":
                self.respond({"kind": "APIVersions", "apiVersion": "v1", "versions": ["v1"]})
            elif path == "/apis":
                self.respond({"kind": "APIGroupList", "apiVersion": "v1", "groups": []})
            elif path == "/api/v1":
                self.respond({"kind": "APIResourceList", "apiVersion": "v1", "groupVersion": "v1", "resources": [
                    {"name": name, "singularName": kind.lower(), "namespaced": namespaced, "kind": kind,
                     "verbs": ["get", "list", "watch"], "shortNames": shortcuts}
                    for name, kind, namespaced, shortcuts in [
                        ("pods", "Pod", True, ["po"]), ("nodes", "Node", False, ["no"]),
                        ("namespaces", "Namespace", False, ["ns"]), ("events", "Event", True, ["ev"]),
                    ]
                ]})
            elif path.endswith("/pods/stream-test/log"):
                self.send_response(200)
                self.send_header("Content-Type", "text/plain")
                self.end_headers()
                self.server.logs_open.set()
                for gate, records in [(self.server.first, [
                    {"level": "INFO", "message": "NOT-A-MATCH"},
                    {"level": "ERROR", "message": "FIRST-ACTUAL-K9S"},
                ]), (self.server.second, [{"level": "ERROR", "message": "SECOND-ACTUAL-K9S"}])]:
                    while not gate.wait(.02):
                        if self.server.stop.is_set(): return
                    for record in records:
                        self.wfile.write(json.dumps(record).encode() + b"\n")
                    self.wfile.flush()
                self.server.stop.wait(20)
            elif path.endswith("/pods/stream-test"):
                self.respond(POD)
            elif path.startswith("/api/v1/") or path.endswith("/customresourcedefinitions"):
                resource = path.rsplit("/", 1)[-1]
                kind, items = {
                    "pods": ("Pod", [POD]), "nodes": ("Node", []),
                    "namespaces": ("Namespace", [{"apiVersion": "v1", "kind": "Namespace", "metadata": {"name": NAMESPACE, "resourceVersion": "1"}}]),
                }.get(resource, ("Event", []))
                if query.get("watch") == ["true"]:
                    self.send_response(200)
                    self.send_header("Content-Type", "application/json")
                    self.end_headers()
                    for item in items:
                        self.wfile.write(json.dumps({"type": "ADDED", "object": item}).encode() + b"\n")
                    # Watch-list clients wait for this bookmark before their
                    # initial cache is considered synced, even for an empty list.
                    self.wfile.write(json.dumps({"type": "BOOKMARK", "object": {
                        "apiVersion": "v1", "kind": kind,
                        "metadata": {"resourceVersion": "1", "annotations": {"k8s.io/initial-events-end": "true"}},
                    }}).encode() + b"\n")
                    self.wfile.flush()
                    self.server.stop.wait(20)
                else:
                    self.respond({"apiVersion": "v1", "kind": kind + "List", "metadata": {"resourceVersion": "1"}, "items": items})
            else:
                self.send_error(404)
        except (BrokenPipeError, ConnectionResetError):
            pass


class RealK9sTests(unittest.TestCase):
    @unittest.skipUnless(shutil.which("k9s") and shutil.which("kubectl"), "requires k9s and kubectl")
    def test_plugin_keyboard_launch_live_filter_preview_and_return(self) -> None:
        with tempfile.TemporaryDirectory(prefix="jgrep-real-k9s-") as directory:
            work = Path(directory)
            api = SyntheticAPI()
            worker = threading.Thread(target=api.serve_forever, daemon=True)
            terminal = None
            try:
                config = work / "config/k9s"
                kubeconfig = work / "kubeconfig"
                kubeconfig.write_text(json.dumps({
                    "apiVersion": "v1", "kind": "Config", "current-context": "synthetic",
                    "clusters": [{"name": "synthetic", "cluster": {"server": f"http://127.0.0.1:{api.server_port}"}}],
                    "contexts": [{"name": "synthetic", "context": {"cluster": "synthetic", "namespace": NAMESPACE, "user": "synthetic"}}],
                    "users": [{"name": "synthetic", "user": {}}],
                }))
                subprocess.run([str(tui.BINARY), "k9s", "install", "--config-dir", str(config)], check=True, capture_output=True, timeout=15)
                terminal = tui.ExplorerTerminal(
                    ["--kubeconfig", str(kubeconfig), "--context", "synthetic", "--namespace", NAMESPACE,
                     "--command", "pods", "--splashless", "--logFile", str(work / "k9s.log")],
                    work, subcommand=None, program=Path(shutil.which("k9s")),
                    environment={"K9S_CONFIG_DIR": str(config), "KUBECONFIG": str(kubeconfig),
                                 "XDG_CONFIG_HOME": str(work / "config"), "XDG_DATA_DIRS": str(work / "data-dirs"),
                                 "XDG_DATA_HOME": str(work / "data"), "XDG_CACHE_HOME": str(work / "cache")},
                )
                worker.start()  # Fork the terminal before starting server threads.
                terminal.wait_for(b"stream-test", rendered=True)
                os.write(terminal.fd, b"J")
                terminal.wait_for(b"waiting for matching logs", rendered=True)
                self.assertTrue(api.logs_open.wait(5))
                os.write(terminal.fd, b"level=ERROR\x0fmessage")
                api.first.set()
                terminal.wait_for(b"FIRST-ACTUAL-K9S", rendered=True)
                self.assertNotIn(b"NOT-A-MATCH", terminal.screen_text())
                os.write(terminal.fd, b"\x07")
                terminal.resize(50, 20)
                api.second.set()
                terminal.wait_for(b"SECOND-ACTUAL-K9S", rendered=True)
                terminal.wait_for(b"stream live: 3 docs", rendered=True)
                os.write(terminal.fd, b"\x13")
                terminal.wait_for(b"saved", rendered=True)
                self.assertEqual((config / "plugins/jgrep/state/saved-filter.jq").read_text(), 'select(.level == "ERROR") | .message\n')
                os.write(terminal.fd, b"\x1b")
                terminal.wait_for(b"<pod>", rendered=True)
                terminal.resize(120, 30)
                os.write(terminal.fd, b"U")
                terminal.wait_for(b"metadata.name", rendered=True)
                terminal.wait_for(b"Preview", rendered=True)
                os.write(terminal.fd, b"\x1b")
                terminal.wait_for(b"<pod>", rendered=True)
                os.write(terminal.fd, b"\r")  # Pod Enter opens its container view.
                terminal.wait_for(b"<containers>", rendered=True)
                os.write(terminal.fd, b"K")
                terminal.wait_for(b"SECOND-ACTUAL-K9S", rendered=True)
                terminal.wait_for(b"Preview", rendered=True)
                os.write(terminal.fd, b"\x1b")
                terminal.wait_for(b"app", rendered=True)
                terminal.finish(b":quit\r")
                self.assertTrue(any(path.endswith("/pods/stream-test/log?follow=true&tailLines=1000") or "/pods/stream-test/log?" in path for _, path in api.requests))
                self.assertTrue(any("/pods/stream-test/log?" in path and "container=app" in path for _, path in api.requests))
                self.assertTrue(any(urlsplit(path).path.endswith("/pods/stream-test") for _, path in api.requests))
            except Exception:
                print("Synthetic API routes:", api.requests)
                if terminal is not None: print("Final screen:", terminal.screen_text().decode()[-3000:])
                log = work / "k9s.log"
                if log.exists(): print(log.read_text()[-3000:])
                raise
            finally:
                api.stop.set()
                if terminal is not None: terminal.close()
                if worker.is_alive(): api.shutdown()
                api.server_close()
                if worker.ident is not None: worker.join(5)


if __name__ == "__main__":
    unittest.main()
