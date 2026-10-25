#!/usr/bin/env python3
"""Recheck recorded commercial sample identities without distributing their bytes."""
import argparse
import hashlib
import json
from pathlib import Path
import struct
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[3]


def verify(steamapps, inspector):
    manifest = json.loads((ROOT / 'docs/validation/a3-texb4-samples.json').read_text())
    with tempfile.TemporaryDirectory(prefix='better-wallpaper-a3-audit-') as directory:
        for index, row in enumerate(manifest['records']):
            package = steamapps / 'workshop/content/431960' / row['source'].removeprefix('workshop-') / 'scene.pkg'
            assert row['size'] <= 128 * 1024 * 1024
            with package.open('rb') as file:
                file.seek(row['offset'])
                data = file.read(row['size'])
            assert len(data) == row['size']
            assert hashlib.sha256(data).hexdigest() == row['sha256']
            assert data[:18] == b'TEXV0005\0TEXI0001\0'
            assert data[46:55] == f"TEXB{row['container_version']:04}\0".encode()
            fields = struct.unpack_from('<' + 'I' * (9 if row['container_version'] == 4 else 8), data, 55)
            images, codec, *rest = fields
            if row['container_version'] == 4:
                assert rest.pop(0) == 0
            mips, width, height, compression, decoded_length, encoded_length = rest
            assert (images, codec, mips, width, height, compression, decoded_length) == (1, 0xffffffff, 1, *row['storage'], 0, 0)
            offset = 55 + len(fields) * 4
            assert offset == row['payload_offset']
            assert offset + encoded_length == len(data)
            media = data[offset:]
            assert hashlib.sha256(media).hexdigest() == row['encoded_sha256']
            texture = Path(directory) / f'{index}.tex'
            texture.write_bytes(data)
            result = subprocess.run([str(inspector), str(texture), '--atlas', '--json'], capture_output=True, text=True, timeout=30)
            if row['parser_status'] == 'rejected_64_mib_payload_limit':
                assert result.returncode and 'exceeds maximum' in result.stderr
            else:
                assert result.returncode == 0, result.stderr
                report = json.loads(result.stdout)
                mip = report['images'][0]['mips'][0]
                assert report['is_video'] and report['editor_metadata'] == []
                assert mip['decoded_sha256'] == row['encoded_sha256']
                assert mip['decoded_bytes'] == len(media)
                movie = Path(directory) / f'{index}.mp4'
                movie.write_bytes(media)
                probe = json.loads(subprocess.check_output(['ffprobe', '-v', 'error', '-show_entries',
                    'format=format_name,duration:stream=codec_name,codec_type,width,height', '-of', 'json', str(movie)], timeout=30))
                assert probe == row['media_probe']
                movie.unlink()
            texture.unlink()
            print(f"{row['source']}: TEXB{row['container_version']} {row['parser_status']}")


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--steamapps', type=Path, required=True)
    parser.add_argument('--scene-inspect', type=Path, required=True)
    args = parser.parse_args()
    verify(args.steamapps.resolve(), args.scene_inspect.resolve())
