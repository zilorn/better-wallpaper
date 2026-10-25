#!/usr/bin/env python3
"""Repeat the bounded local scene-member occurrence survey, without extraction."""
import argparse
from collections import Counter
import hashlib
import json
import mmap
from pathlib import Path
import struct


def survey(root):
    paths = sorted(p for p in root.glob('*/scene.pkg') if not p.is_symlink() and not p.parent.is_symlink())
    if len(paths) > 64:
        raise ValueError('Survey is limited to 64 explicitly selected packages')
    authored, enabled, fields = Counter(), Counter(), Counter()
    rows = []
    for path in paths:
        with path.open('rb') as stream, mmap.mmap(stream.fileno(), 0, access=mmap.ACCESS_READ) as data:
            cursor = 0
            def word():
                nonlocal cursor
                n = struct.unpack_from('<I', data, cursor)[0]
                cursor += 4
                return n
            def string():
                nonlocal cursor
                n = word()
                if n > 512 or cursor + n > len(data):
                    raise ValueError('Invalid package string')
                s = data[cursor:cursor+n].decode('utf-8')
                cursor += n
                return s
            version = string()
            if len(version) != 8 or not version.startswith('PKGV') or not version[4:].isdigit():
                raise ValueError('Invalid package header')
            count = word()
            if count > 4096:
                raise ValueError('Package index exceeds limit')
            entries = [(string(), word(), word()) for _ in range(count)]
            matches = [(o, n) for name, o, n in entries if name == 'scene.json']
            if len(matches) != 1:
                raise ValueError('Expected one scene.json member')
            offset, size = matches[0]
            offset += cursor
            if size > 2*1024*1024 or offset+size > len(data):
                raise ValueError('Invalid or over-limit scene member')
            raw = data[offset:offset+size]
            scene = json.loads(raw)
            effects = Counter()
            for obj in scene.get('objects', []):
                for effect in obj.get('effects', []):
                    visible = effect.get('visible', True)
                    name = effect.get('file', '?')
                    if visible is not False:
                        effects[name] += 1
                    if (visible.get('value', True) if isinstance(visible, dict) else visible) is not False:
                        enabled[name] += 1
                for key in ['origin', 'angles', 'scale', 'size', 'alpha']:
                    if isinstance(obj.get(key), dict) and 'animation' in obj[key]:
                        fields[key] += 1
            authored.update(effects)
            rows.append({'project': path.parent.name, 'scene_sha256': hashlib.sha256(raw).hexdigest(),
                         'scene_offset': offset, 'scene_bytes': size, 'effects': dict(effects)})
    return {'selection': 'sorted */scene.pkg in local workshop root, scene.json only',
            'projects': rows, 'effects': dict(authored), 'transform_animations': dict(fields),
            'default_enabled_effects': dict(enabled),
            'count_boundary': 'Authored excludes literal visible:false; default-enabled also unwraps visible.value. Counts identify skipped operations, not whole-project visual failure rates.'}


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--root', type=Path, required=True)
    parser.add_argument('--write', type=Path)
    args = parser.parse_args()
    result = survey(args.root)
    if args.write:
        args.write.write_text(json.dumps(result, indent=2)+'\n')
    else:
        baseline = json.loads((Path(__file__).resolve().parents[3]/'docs/validation/a4-scene-survey.json').read_text())
        if result != baseline:
            raise SystemExit('Survey differs; do not overwrite historical evidence silently')
    print(f'Verified or wrote {len(result["projects"])} scene-member identities and occurrence counts')
