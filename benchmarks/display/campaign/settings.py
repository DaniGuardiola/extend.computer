"""Apply benchmark settings through the authenticated source host and app path."""
import argparse,json
from pathlib import Path
from bootstrap import ROOT
from client import Client
from launch import connection_arguments

def configuration(profile, fps=60, quality="balanced"):
    args=connection_arguments("account",profile,3600,fps,quality)
    return {"mode":"extend","width":int(args[2]),"height":int(args[3]),"fps":fps,"bitrate":int(args[-1]),
            "retina":args[5]=="retina","auto_size":False,"codec":"hevc","separate_cursor":True}

def main():
    p=argparse.ArgumentParser(description=__doc__)
    p.add_argument('--pairing',type=Path,default=ROOT/'.cache/campaign/source-pairing.json')
    p.add_argument('--display-profile',required=True)
    p.add_argument('--fps',type=int,choices=[30,60,120],default=60)
    p.add_argument('--quality',choices=['responsive','balanced','detail'],default='balanced')
    a=p.parse_args()
    try:config=configuration(a.display_profile,a.fps,a.quality)
    except ValueError as error:p.error(str(error))
    result=Client(a.pairing).request('/display-settings',{'product':'extend','config':config})
    print('Display settings applied on both devices: '+json.dumps(result['configuration'],sort_keys=True))
if __name__=='__main__':main()
