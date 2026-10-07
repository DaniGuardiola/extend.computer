#!/usr/bin/env python3
"""Summarize a sanitized benchmark JSONL file; never copy arbitrary fields."""
import argparse
import json
from pathlib import Path

from benchmark_core import ParseError, summarize


def read_records(path):
    records = []
    with path.open() as stream:
        for number, line in enumerate(stream, 1):
            if not line.strip():
                continue
            try:
                value = json.loads(line)
            except (ValueError, TypeError):
                raise ParseError(f'Invalid JSON record at line {number}') from None
            if not isinstance(value, dict):
                raise ParseError(f'Invalid record type at line {number}')
            records.append(value)
    return records


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('input', type=Path)
    parser.add_argument('--output', type=Path)
    args = parser.parse_args()
    try:
        result = summarize(read_records(args.input))
        if args.output:
            with args.output.open('x') as stream:
                json.dump(result, stream, indent=2, allow_nan=False)
                stream.write('\n')
        else:
            print(json.dumps(result, indent=2, allow_nan=False))
    except (OSError, ValueError, KeyError, TypeError):
        parser.exit(1, 'Summary failed: invalid input or output unavailable\n')
    return 0


if __name__ == '__main__':
    raise SystemExit(main())
