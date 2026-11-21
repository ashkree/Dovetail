#!/usr/bin/env python3
"""Fake HTTP MCP server that always answers 401 (needs auth). Usage: auth_server.py PORT"""
import sys
from http.server import BaseHTTPRequestHandler, HTTPServer


class H(BaseHTTPRequestHandler):
    def _r(self):
        n = int(self.headers.get("content-length") or 0)
        self.rfile.read(n)
        self.send_response(401)
        self.send_header("WWW-Authenticate", 'Bearer realm="spike"')
        self.send_header("content-length", "0")
        self.end_headers()

    do_POST = do_GET = _r

    def log_message(self, *a):
        pass


HTTPServer(("127.0.0.1", int(sys.argv[1])), H).serve_forever()
