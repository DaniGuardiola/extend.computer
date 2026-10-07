#!/usr/bin/env python3
"""Explicit ADB readiness check. Does not launch Camera, record, or change settings."""
import argparse
import json
import subprocess


def check(serial,adb='adb'):
    def read(args):
        r=subprocess.run([adb,'-s',serial,*args],capture_output=True,text=True,timeout=10,check=True)
        return r.stdout.strip()
    return {'adb_connected':read(['get-state'])=='device','model':read(['shell','getprop','ro.product.model']),
            'recording_control':'not_calibrated','capture_rate':'not_verified','optical_decoder':'pending',
            'physical_position':'operator_required'}


def main():
    p=argparse.ArgumentParser(description=__doc__);p.add_argument('--serial',required=True);p.add_argument('--adb',default='adb');args=p.parse_args()
    try:print(json.dumps(check(args.serial,args.adb),indent=2))
    except (OSError,subprocess.SubprocessError):p.exit(1,'ADB readiness failed; authorize the explicit device and check connection\n')

if __name__=='__main__':main()
