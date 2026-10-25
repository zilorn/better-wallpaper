#!/usr/bin/env python3
"""Bounded local script survey; retain identities/counts, never commercial source."""
import argparse
from collections import Counter
import hashlib
import json
import mmap
from pathlib import Path
import re
import struct


def survey(root):
    paths = sorted(p for p in root.glob('*/scene.pkg') if not p.is_symlink() and not p.parent.is_symlink())
    if len(paths) > 64:
        raise ValueError('Select at most 64 packages')
    rows, fields, apis, language = [], Counter(), Counter(), Counter()
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
                if n > 512 or cursor+n > len(data):
                    raise ValueError('Invalid package string')
                value = data[cursor:cursor+n].decode()
                cursor += n
                return value
            version = string()
            if not re.fullmatch(r'PKGV\d{4}', version):
                raise ValueError('Invalid package version')
            count = word()
            if count > 4096:
                raise ValueError('Package index budget exceeded')
            entries = [(string(), word(), word()) for _ in range(count)]
            matches = [(o,n) for name,o,n in entries if name == 'scene.json']
            if len(matches) != 1:
                raise ValueError('Expected one scene member')
            offset, size = matches[0]
            offset += cursor
            if size > 2*1024*1024 or offset+size > len(data):
                raise ValueError('Scene member exceeds bounds')
            raw = data[offset:offset+size]
        bindings = []
        def walk(value, location, depth=0):
            if depth > 64:
                raise ValueError('Nesting budget exceeded')
            if isinstance(value, dict):
                source = value.get('script')
                if isinstance(source, str):
                    if len(bindings) >= 4096 or len(source.encode()) > 1024*1024:
                        raise ValueError('Script survey budget exceeded')
                    calls = sorted(set(re.findall(r'\b(?:engine|thisLayer|thisScene|thisObject|input|Math|Date|shared)\.[A-Za-z_][A-Za-z_0-9]*', source)))
                    syntax = sorted(name for name,pattern in {
                        'update': r'export\s+function\s+update\b', 'init': r'export\s+function\s+init\b',
                        'destroy': r'export\s+function\s+destroy\b', 'properties': r'applyUserProperties',
                        'scriptProperties': r'createScriptProperties', 'loops': r'\b(?:for|while)\s*\(',
                        'Date': r'\bDate\b', 'Vec3': r'\bVec3\b', 'shared': r'\bshared\.',
                    }.items() if re.search(pattern, source))
                    bindings.append({'path': location, 'bytes': len(source.encode()),
                                     'sha256': hashlib.sha256(source.encode()).hexdigest(),
                                     'candidate_host_members': calls, 'candidate_syntax': syntax})
                    fields[location.split('.')[-1]] += 1
                    apis.update(calls)
                    language.update(syntax)
                for key,child in value.items():
                    walk(child, location+'.'+key, depth+1)
            elif isinstance(value, list):
                for i,child in enumerate(value):
                    walk(child, location+f'[{i}]', depth+1)
        walk(json.loads(raw), 'scene')
        rows.append({'project': path.parent.name, 'scene_sha256': hashlib.sha256(raw).hexdigest(),
                     'scene_offset': offset, 'scene_bytes': size, 'bindings': bindings})
    return {'selection': 'sorted non-symlink */scene.pkg; scene.json only, at most 64 packages',
            'boundary': 'Regex candidates include comments and dead code; counts are script bindings, not executions, semantic dependencies or playback success. No external JS modules inspected.',
            'projects': rows, 'binding_count': sum(len(row['bindings']) for row in rows),
            'fields': dict(sorted(fields.items())), 'candidate_host_members': dict(sorted(apis.items())),
            'candidate_syntax': dict(sorted(language.items()))}


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--root', required=True, type=Path)
    parser.add_argument('--write', type=Path)
    args = parser.parse_args()
    result = survey(args.root)
    if args.write:
        args.write.write_text(json.dumps(result, indent=2)+'\n')
    else:
        baseline = json.loads((Path(__file__).resolve().parents[3]/'docs/validation/a7-script-survey.json').read_text())
        if result != baseline:
            raise SystemExit('Survey differs; preserve historical evidence')
    print(f'{len(result["projects"])} projects, {result["binding_count"]} script bindings')
