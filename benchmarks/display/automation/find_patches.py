import json,sys
import numpy as np
from PIL import Image

def find(path):
    im=np.array(Image.open(path))[:,:,:3].astype(int)
    r,g,b=im.transpose(2,0,1)
    masks=[(r>150)&(b>150)&(g<120)&(r>g+60)&(b>g+60),(g>150)&(b>150)&(r<g-20)&(r<b-20),(r>150)&(g<110)&(b<110),((g>150)&(g>r+20)&(g>b+8))|((g>170)&(b>170)&(r<g-35)&(r<b-35))]
    centers=[]
    shapes={}
    for index, mask in enumerate(masks):
        ys,xs=np.nonzero(mask);remaining=set(zip(xs.tolist(),ys.tolist()));components=[]
        accepted_pixels=set()
        while remaining:
            point=remaining.pop();stack=[point];points=[point]
            while stack:
                x,y=stack.pop()
                for p in [(x-1,y),(x+1,y),(x,y-1),(x,y+1)]:
                    if p in remaining:remaining.remove(p);stack.append(p);points.append(p)
            if len(points)<30 or len(points)>1600:continue
            x,y=zip(*points);w=max(x)-min(x)+1;h=max(y)-min(y)+1
            if .45<w/h<2.2:
                center=[sum(x)/len(x),sum(y)/len(y)]
                components.append(center)
                shapes[tuple(center)]=(w,h,len(points))
                if index == 1:accepted_pixels.update(points)
        if index == 1:
            strict = mask & (b > g + 8) & (g > r + 60)
            ys,xs=np.nonzero(strict);remaining=set(zip(xs.tolist(),ys.tolist()))
            while remaining:
                point=remaining.pop();stack=[point];points=[point]
                while stack:
                    x,y=stack.pop()
                    for p in [(x-1,y),(x+1,y),(x,y-1),(x,y+1)]:
                        if p in remaining:remaining.remove(p);stack.append(p);points.append(p)
                # The strict mask is a subset of the broad cyan mask. A
                # fragment of an already accepted marker is not a second
                # marker; keep strict fallback only for rejected components.
                if any(point in accepted_pixels for point in points):continue
                if not 30 <= len(points) <= 1600:continue
                x,y=zip(*points);w=max(x)-min(x)+1;h=max(y)-min(y)+1
                if .45<w/h<2.2:
                    center=[sum(x)/len(x),sum(y)/len(y)]
                    components.append(center)
                    shapes[tuple(center)]=(w,h,len(points))
        centers.append(components)
    patches=[]
    for tl in centers[0]:
        for tr in centers[1]:
            w=tr[0]-tl[0]
            # A fixed camera can see a rotated screen. Permit up to about
            # 15 degrees; the other corners still have to close the same quad.
            if not 80<w<800 or abs(tr[1]-tl[1])>w*.27:continue
            for bl in centers[2]:
                h=bl[1]-tl[1]
                if not 30<h<300 or not 1.8<w/h<3.8 or abs(bl[0]-tl[0])>h*.35:continue
                for br in centers[3]:
                    if abs((br[0]-bl[0])-w)>w*.08 or abs((br[1]-tr[1])-h)>h*.15:continue
                    # All four fiducials have the same source size. Perspective,
                    # washout and threshold erosion permit bounded variation;
                    # tiny text/cell fragments cannot substitute for a corner.
                    sizes=[shapes[tuple(point)] for point in (tl,tr,bl,br)]
                    if any(max(size[axis] for size in sizes)>limit*min(size[axis] for size in sizes) for axis,limit in ((0,2),(1,2),(2,4))):continue
                    patches.append([tl,tr,bl,br])
    def score(p):
        tl,tr,bl,br=p;return abs((tr[0]-tl[0])-(br[0]-bl[0]))+abs((bl[1]-tl[1])-(br[1]-tr[1]))
    unique=[]
    for candidate in sorted(patches,key=score):
        center=np.mean(candidate,axis=0)
        if any(np.linalg.norm(center-np.mean(other,axis=0))<.5*max(candidate[1][0]-candidate[0][0],other[1][0]-other[0][0]) for other in unique):continue
        unique.append(candidate)
    return sorted(unique,key=lambda p:p[0][0])
if __name__=='__main__':
    patches=find(sys.argv[1]);print(json.dumps(patches));
    if not patches:raise SystemExit('No readable timing patch markers')
