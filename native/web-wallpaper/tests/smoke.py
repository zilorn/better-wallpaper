"""Run explicitly in a niri session; never changes saved wallpaper configuration.

python3 native/web-wallpaper/tests/smoke.py [path/to/better-wallpaper-web]
"""

import http.server
import json
import pathlib
import subprocess
import sys
import tempfile
import threading
import time

HTML = b"""<!doctype html>
<body style="margin:0;background:#15243b;color:white">
<canvas id="c" width="400" height="240"></canvas>
<script>
const ctx = c.getContext('2d');
let t = 0;
function draw() {
    ctx.fillStyle = '#15243b'; ctx.fillRect(0, 0, 400, 240);
    ctx.fillStyle = '#70b9ee'; ctx.fillRect(t++ % 300, 80, 70, 70);
    requestAnimationFrame(draw);
}
draw();
setInterval(() => fetch('/tick'), 100);
const gl = document.createElement('canvas').getContext('webgl');
fetch('/webgl?value=' + Boolean(gl));
window.wallpaperPropertyListener = {
    applyUserProperties: p => fetch('/properties?value=' + p.tint.value),
    setPaused: p => fetch('/pause?value=' + p)
};
fetch('/bridge?value=' + typeof wallpaperRegisterAudioListener);
</script>"""


def main():
    requests = []

    class Handler(http.server.BaseHTTPRequestHandler):
        def do_GET(self):
            requests.append(self.path)
            self.send_response(200)
            self.send_header('Content-Type', 'text/html')
            self.end_headers()
            self.wfile.write(HTML if self.path == '/index.html' else b'ok')

        def log_message(self, *args):
            pass

    server = http.server.ThreadingHTTPServer(('127.0.0.1', 0), Handler)
    worker = threading.Thread(target=server.serve_forever, daemon=True)
    worker.start()
    player = sys.argv[1] if len(sys.argv) > 1 else 'target/web-wallpaper/better-wallpaper-web'

    with tempfile.TemporaryDirectory(prefix='better-wallpaper-web-smoke-') as temporary:
        log_path = pathlib.Path(temporary) / 'renderer.log'
        with log_path.open('w') as log:
            process = subprocess.Popen(
                [player], stdin=subprocess.PIPE, stdout=log, stderr=log, text=True
            )

            def send(message):
                process.stdin.write(json.dumps(message) + '\n')
                process.stdin.flush()

            def ticks():
                return requests.count('/tick')

            try:
                send({
                    'url': f'http://127.0.0.1:{server.server_port}/index.html',
                    'properties': {'tint': {'value': 'blue'}},
                    'outputs': [], 'muted': True, 'paused': False,
                })
                deadline = time.monotonic() + 12
                while time.monotonic() < deadline and ticks() < 5 and process.poll() is None:
                    time.sleep(0.1)
                assert process.poll() is None, f'Renderer exited: {process.returncode}'
                assert '/properties?value=blue' in requests, requests
                assert '/bridge?value=function' in requests, requests
                assert '/webgl?value=true' in requests, requests
                assert ticks() >= 5, requests

                send({'paused': True})
                time.sleep(1)
                frozen = ticks()
                time.sleep(1)
                assert ticks() == frozen, 'Timers continued while frozen'

                send({'paused': False})
                time.sleep(1)
                assert ticks() > frozen, 'Timers did not resume'

                process.stdin.close()
                process.wait(timeout=5)
                assert process.returncode == 0, process.returncode
                print('PASS: page load, WebGL, timers, properties, compatibility bridge, '
                      'pause freeze, resume, stdin-close cleanup')
            finally:
                if process.poll() is None:
                    process.kill()
                    process.wait()
                server.shutdown()
                server.server_close()
                worker.join()
                log.flush()
                print(log_path.read_text())


if __name__ == '__main__':
    main()
