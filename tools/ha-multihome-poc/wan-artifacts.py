#!/usr/bin/env python3
"""Temporary VPN-only source/binary exchange for the two-host qualification."""
import argparse
import ipaddress
import json
import os
from pathlib import Path
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import threading


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--config', required=True)
    args = parser.parse_args()
    config = json.loads(Path(args.config).read_text())
    overlay = ipaddress.ip_network('100.64.0.0/10')
    bind = config['observer_ip']
    peers = {site['host'] for site in config['sites']}
    assert all(ipaddress.ip_address(ip) in overlay for ip in peers | {bind})
    root = Path(args.config).resolve().parent
    paths = {'/source.tgz': 'source.tgz', '/binary': 'moli-ha-site-linux',
             '/site-a.json': 'site-a.json', '/site-b.json': 'site-b.json'}
    slots = threading.BoundedSemaphore(8)

    class Handler(BaseHTTPRequestHandler):
        def log_message(self, *_args):
            pass

        def setup(self):
            super().setup()
            self.connection.settimeout(10)

        def allowed(self):
            if self.client_address[0] not in peers:
                self.send_error(403)
                return False
            return True

        def do_GET(self):
            if not self.allowed():
                return
            name = paths.get(self.path)
            if name is None:
                self.send_error(404)
                return
            if self.path.startswith('/site-'):
                index = 0 if self.path == '/site-a.json' else 1
                if self.client_address[0] != config['sites'][index]['host']:
                    self.send_error(403)
                    return
            path = root / name
            if not path.is_file():
                self.send_error(404)
                return
            self.send_response(200)
            self.send_header('Content-Length', str(path.stat().st_size))
            self.send_header('Cache-Control', 'no-store')
            self.end_headers()
            with path.open('rb') as source:
                while chunk := source.read(65536):
                    self.wfile.write(chunk)

        def do_POST(self):
            if not self.allowed():
                return
            if self.path != '/binary' or self.client_address[0] != config['sites'][0]['host']:
                self.send_error(403)
                return
            size = int(self.headers.get('Content-Length', '0'))
            if not 4 <= size <= 20 * 1024 * 1024:
                self.send_error(413)
                return
            temporary = root / 'linux-binary.part'
            try:
                descriptor = os.open(temporary, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
                with os.fdopen(descriptor, 'wb') as target:
                    left = size
                    while left:
                        chunk = self.rfile.read(min(left, 65536))
                        if not chunk:
                            raise ValueError('truncated binary')
                        target.write(chunk)
                        left -= len(chunk)
                with temporary.open('rb') as source:
                    if source.read(4) != b'\x7fELF':
                        raise ValueError('not an ELF binary')
                temporary.replace(root / paths['/binary'])
                self.send_response(201)
                self.send_header('Content-Length', '0')
                self.end_headers()
                print('LINUX_BINARY_RECEIVED', size, flush=True)
            finally:
                temporary.unlink(missing_ok=True)

    class Server(ThreadingHTTPServer):
        daemon_threads = True

        def process_request(self, request, address):
            if not slots.acquire(False):
                request.close()
                return
            try:
                super().process_request(request, address)
            except BaseException:
                slots.release()
                raise

        def process_request_thread(self, request, address):
            try:
                super().process_request_thread(request, address)
            finally:
                slots.release()

    server = Server((bind, config['artifact_port']), Handler)
    timer = threading.Timer(1800, server.shutdown)
    timer.daemon = True
    timer.start()
    print('VPN_ARTIFACTS_READY', flush=True)
    try:
        server.serve_forever()
    finally:
        timer.cancel()
        server.server_close()


if __name__ == '__main__':
    main()
