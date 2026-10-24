#!/usr/bin/env python3
"""Serve one signed test artifact to one authorized LAN device, without directory access."""
import argparse
from http.server import BaseHTTPRequestHandler, HTTPServer
from pathlib import Path
import shutil

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('artifact', type=Path)
parser.add_argument('--bind', required=True)
parser.add_argument('--peer', required=True)
parser.add_argument('--port', type=int, default=53772)
args = parser.parse_args()
artifact = args.artifact.resolve()

class Handler(BaseHTTPRequestHandler):
    def do_GET(self):
        if self.client_address[0] != args.peer or self.path != '/display-test.zip':
            self.send_error(404)
            return
        self.send_response(200)
        self.send_header('Content-Type', 'application/zip')
        self.send_header('Content-Length', str(artifact.stat().st_size))
        self.end_headers()
        with artifact.open('rb') as file:
            shutil.copyfileobj(file, self.wfile)
    def log_message(self, *args):
        pass

print('Serving signed test artifact to selected LAN device only.', flush=True)
HTTPServer((args.bind, args.port), Handler).serve_forever()
