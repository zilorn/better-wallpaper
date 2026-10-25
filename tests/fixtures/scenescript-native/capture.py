#!/usr/bin/env python3
"""Capture owned probes in an isolated, pre-provisioned Wine app/prefix directory.

See README.md. Never writes repository references; compare outputs before copying.
"""
import argparse
import ctypes as C
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import time
import sys
sys.dont_write_bytecode = True


def capture(work, selected=None, capture_name='recapture'):
    source = Path(__file__).resolve().parent
    manifest = json.loads((source / 'manifest.json').read_text())
    exe = work / 'app/wallpaper64.exe'
    if hashlib.sha256(exe.read_bytes()).hexdigest() != manifest['program']['exe_sha256']:
        raise SystemExit('Executable identity differs from the reference build')
    for name, digest in manifest['sha256'].items():
        if not name.startswith('captures/') and hashlib.sha256((source / name).read_bytes()).hexdigest() != digest:
            raise SystemExit(f'Owned input differs from manifest: {name}')
    if not (work / 'pfx/system.reg').is_file():
        raise SystemExit('Missing isolated Wine prefix: see README.md')
    if Path(capture_name).name != capture_name or capture_name in ['.', '..']:
        raise ValueError('Capture name must be a single new directory name')
    output = work / capture_name
    output.mkdir()  # Refuse to overwrite a prior capture.
    names = sorted(manifest['native_cases'])
    if selected:
        if not set(selected).issubset(names):
            raise SystemExit('Unknown probe selection')
        names = [name for name in names if name in selected]
    for name in names:
        shutil.copytree(source / name, output / name)

    xlib = C.CDLL('libX11.so.6')
    display_type, window_type = C.c_void_p, C.c_ulong
    xlib.XOpenDisplay.argtypes = [C.c_char_p]
    xlib.XOpenDisplay.restype = display_type
    xlib.XDefaultRootWindow.argtypes = [display_type]
    xlib.XDefaultRootWindow.restype = window_type
    xlib.XFetchName.argtypes = [display_type, window_type, C.POINTER(C.c_char_p)]
    xlib.XQueryTree.argtypes = [display_type, window_type, C.POINTER(window_type), C.POINTER(window_type), C.POINTER(C.POINTER(window_type)), C.POINTER(C.c_uint)]
    xlib.XGetGeometry.argtypes = [display_type, window_type, C.POINTER(window_type), C.POINTER(C.c_int), C.POINTER(C.c_int), C.POINTER(C.c_uint), C.POINTER(C.c_uint), C.POINTER(C.c_uint), C.POINTER(C.c_uint)]
    for method in ['XRaiseWindow', 'XUnmapWindow']:
        getattr(xlib, method).argtypes = [display_type, window_type]
    xlib.XFlush.argtypes = [display_type]
    xlib.XWarpPointer.argtypes = [display_type, window_type, window_type, C.c_int, C.c_int, C.c_uint, C.c_uint, C.c_int, C.c_int]
    xlib.XCloseDisplay.argtypes = [display_type]
    xlib.XFree.argtypes = [C.c_void_p]
    xlib.XGetImage.argtypes = [display_type, window_type, C.c_int, C.c_int, C.c_uint, C.c_uint, C.c_ulong, C.c_int]
    xlib.XGetImage.restype = C.c_void_p
    xlib.XGetPixel.argtypes = [C.c_void_p, C.c_int, C.c_int]
    xlib.XGetPixel.restype = C.c_ulong
    xlib.XDestroyImage.argtypes = [C.c_void_p]

    def windows(window=None):
        window = window or xlib.XDefaultRootWindow(display)
        name = C.c_char_p()
        xlib.XFetchName(display, window, C.byref(name))
        root, parent = window_type(), window_type()
        children, count = C.POINTER(window_type)(), C.c_uint()
        x, y, width, height, border, bits = C.c_int(), C.c_int(), C.c_uint(), C.c_uint(), C.c_uint(), C.c_uint()
        xlib.XGetGeometry(display, window, C.byref(root), C.byref(x), C.byref(y), C.byref(width), C.byref(height), C.byref(border), C.byref(bits))
        label = name.value.decode(errors='replace') if name.value else ''
        if name:
            xlib.XFree(name)
        yield window, label, width.value, height.value
        xlib.XQueryTree(display, window, C.byref(root), C.byref(parent), C.byref(children), C.byref(count))
        try:
            for index in range(count.value):
                yield from windows(children[index])
        finally:
            if children:
                xlib.XFree(children)

    with (output / 'launch.log').open('w') as log:
        read_fd, write_fd = os.pipe()
        server = subprocess.Popen(['Xvfb', '-displayfd', str(write_fd), '-screen', '0', '640x480x24', '-nolisten', 'tcp'], pass_fds=(write_fd,), stdout=log, stderr=log)
        os.close(write_fd)
        import select
        if not select.select([read_fd], [], [], 10)[0]:
            server.terminate()
            server.wait(timeout=10)
            os.close(read_fd)
            raise RuntimeError('Xvfb startup timed out')
        name = ':' + os.read(read_fd, 32).decode().strip()
        os.close(read_fd)
        env = dict(os.environ, DISPLAY=name, WINEPREFIX=str(work / 'pfx'), WINEDEBUG='-all', LIBGL_ALWAYS_SOFTWARE='1', WINEDLLOVERRIDES='d3d11,dxgi=b')
        env.pop('WAYLAND_DISPLAY', None)
        display = xlib.XOpenDisplay(name.encode())
        try:
            if not display:
                raise RuntimeError('X display unavailable')
            for probe in names:
                fixture = output / probe / 'scene.pkg'
                process = subprocess.Popen(['wine', str(exe), '-silent', '-control', 'openWallpaper', '-file', 'Z:' + str(fixture).replace('/', '\\'), '-playInWindow', 'A7 SceneScript Probe', '-width', '128', '-height', '128'], cwd=exe.parent, env=env, stdout=log, stderr=log)
                deadline = time.monotonic() + 20
                matches = []
                while time.monotonic() < deadline:
                    matches = [row for row in windows() if row[1:] == ('A7 SceneScript Probe', 128, 128)]
                    if matches:
                        break
                    time.sleep(.2)
                if len(matches) != 1:
                    raise RuntimeError(f'{probe}: expected exactly one 128x128 client')
                window = matches[0][0]
                time.sleep(3)
                for other, label, _, _ in windows():
                    if label == 'Wallpaper UI':
                        xlib.XUnmapWindow(display, other)
                xlib.XRaiseWindow(display, window)
                xlib.XFlush(display)
                point = manifest['pointer'].get(probe, [64, 64])
                if probe == 'parallax-smooth':
                    xlib.XWarpPointer(display, 0, window, 0, 0, 0, 0, 64, 64)
                    xlib.XFlush(display)
                    time.sleep(2)
                    started = time.monotonic()
                    xlib.XWarpPointer(display, 0, window, 0, 0, 0, 0, 96, 64)
                    xlib.XFlush(display)
                    trajectory = []
                    while time.monotonic() - started < 1.5:
                        before = time.monotonic() - started
                        image = xlib.XGetImage(display, window, 0, 0, 128, 128, C.c_ulong(-1).value, 2)
                        try:
                            columns = [x for x in range(128) if (xlib.XGetPixel(image, x, 64) >> 16) & 255 > 200]
                        finally:
                            xlib.XDestroyImage(image)
                        after = time.monotonic() - started
                        trajectory.append({'before':before, 'after':after, 'center_x':(min(columns)+max(columns))/2 if columns else None})
                        time.sleep(1/60)
                    (output / 'trajectory.json').write_text(json.dumps(trajectory, indent=2)+'\n')
                xlib.XWarpPointer(display, 0, window, 0, 0, 0, 0, point[0], point[1])
                xlib.XFlush(display)
                time.sleep(2)
                subprocess.run(['import', '-window', str(window), str(output / (probe + '.png'))], env=env, check=True, timeout=5)
                subprocess.run(['wineserver', '-k'], env=env, check=True, timeout=10)
                subprocess.run(['wineserver', '-w'], env=env, check=True, timeout=10)
                process.wait(timeout=10)
                print(f'Captured {probe}', flush=True)
        finally:
            subprocess.run(['wineserver', '-k'], env=env, timeout=10)
            if display:
                xlib.XCloseDisplay(display)
            server.terminate()
            server.wait(timeout=10)


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--work', type=Path, required=True)
    parser.add_argument('--only', action='append')
    parser.add_argument('--capture-name', default='recapture')
    args = parser.parse_args()
    capture(args.work.resolve(), args.only, args.capture_name)
