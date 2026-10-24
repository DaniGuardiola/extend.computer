#!/usr/bin/env python3
"""Verify the generated receiver pattern in a tablet screenshot (not optical latency)."""
import argparse, json
from PIL import Image

def verify(path):
    image=Image.open(path).convert('RGB')
    scale=min(image.width/1280,image.height/800)
    left=(image.width-1280*scale)/2;top=(image.height-800*scale)/2
    sample=lambda x,y:image.getpixel((round(left+x*scale),round(top+y*scale)))
    colors=[sample(x,250) for x in (213,640,1066)]
    for channel,color in enumerate(colors):
        assert color[channel]>200 and all(c<60 for i,c in enumerate(color) if i!=channel), 'Incorrect color panel: '+str(colors)
    checker=[sum(sample(x,650))/3 for x in (20,60,100,140)]
    assert min(checker)<60 and max(checker)>200 and all(abs(a-b)>150 for a,b in zip(checker,checker[1:])), 'Checkerboard missing: '+str(checker)
    return {'status':'passed','panels':colors,'checker':checker,'size':[image.width,image.height]}

if __name__=='__main__':
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('image');args=parser.parse_args()
    print(json.dumps(verify(args.image)))
