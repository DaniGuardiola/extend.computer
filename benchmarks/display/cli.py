"""Analyze immutable normalized runs, or compare equivalent measurements."""
import argparse
import json
from .core import load_run, compare


def main():
    p=argparse.ArgumentParser(description=__doc__)
    sub=p.add_subparsers(dest="action",required=True)
    a=sub.add_parser("analyze");a.add_argument("run")
    c=sub.add_parser("compare");c.add_argument("left");c.add_argument("right")
    args=p.parse_args()
    try:
        result=load_run(args.run) if args.action=="analyze" else compare(load_run(args.left),load_run(args.right))
        print(json.dumps(result,indent=2,allow_nan=False))
        return 0 if args.action=="analyze" or result["comparable"] else 2
    except (ValueError,OSError,KeyError,TypeError) as error:
        p.exit(1,f"Benchmark rejected: {error}\n")

if __name__=="__main__": raise SystemExit(main())
