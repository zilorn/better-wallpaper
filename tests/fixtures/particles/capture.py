#!/usr/bin/env python3
"""Reuse the isolated native window capture harness for three static A6 probes."""
import argparse
import importlib.util
import json
from pathlib import Path
import sys
sys.dont_write_bytecode = True
ROOT = Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location('motion_capture', ROOT.parent / 'motion-native/capture.py')
helper = importlib.util.module_from_spec(spec)
spec.loader.exec_module(helper)
# The shared harness resolves its fixture source relative to __file__.
helper.__file__ = __file__

if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('work', type=Path)
    args = parser.parse_args()
    helper.capture(args.work.resolve(), json.loads((ROOT / 'manifest.json').read_text())['native_cases'])
