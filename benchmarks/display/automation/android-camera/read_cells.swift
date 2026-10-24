import Foundation
import AVFoundation
import CoreVideo
func locate(_ expected:[Double],_ color:Int,width:Int,height:Int,searchRadius:Int = 16,matches:(Int,Int,Int)->Bool)->[Double]? {
    let cx=Int(expected[0].rounded()),cy=Int(expected[1].rounded()),radius=searchRadius,side=2*searchRadius+1
    guard cx-radius>=0 && cy-radius>=0 && cx+radius<width && cy+radius<height else{return nil}
    var remaining=Set<Int>()
    for y in 0..<side {for x in 0..<side {
        if matches(color,cx-radius+x,cy-radius+y){remaining.insert(y*side+x)}
    }}
    var best:[Double]?,distance=Double.infinity
    while let seed=remaining.first {
        remaining.remove(seed);var stack=[seed],points=[seed]
        while let point=stack.popLast() {
            let x=point%side,y=point/side
            for (dx,dy) in [(-1,0),(1,0),(0,-1),(0,1)] {
                let nx=x+dx,ny=y+dy
                if nx<0 || ny<0 || nx>=side || ny>=side {continue}
                let next=ny*side+nx
                if remaining.remove(next) != nil {stack.append(next);points.append(next)}
            }
        }
        // Smaller logical resolutions enlarge the patch on the receiving
        // panel. Accept a fully contained marker up to the search area's
        // interior size; clipped components and displacement still reject.
        if points.count<25 || points.count>(side-2)*(side-2) {continue}
        let xs=points.map{$0%side},ys=points.map{$0/side}
        // A clipped component can appear falsely close to the expected center.
        if xs.min()==0 || ys.min()==0 || xs.max()==side-1 || ys.max()==side-1 {continue}
        let w=xs.max()!-xs.min()!+1,h=ys.max()!-ys.min()!+1
        if Double(w)/Double(h)<0.45 || Double(w)/Double(h)>2.2 {continue}
        let x=Double(xs.reduce(0,+))/Double(points.count)+Double(cx-radius)
        let y=Double(ys.reduce(0,+))/Double(points.count)+Double(cy-radius)
        let d=hypot(x-expected[0],y-expected[1])
        if d<=6 && d<distance {best=[x,y];distance=d}
    }
    return best
}

func cursorComponent(_ columns:[Int], span:Double)->[String:Double]? {
    guard span.isFinite, span>0, let first=columns.first, let last=columns.last,
          columns.count>=4, last-first>=6, last-first<=26,
          zip(columns,columns.dropFirst()).allSatisfy({$1-$0==2}) else {return nil}
    return ["x":Double(columns.reduce(0,+))/Double(columns.count),
            "width":Double(last-first+2), "spatial_bound":max(2,364/span)]
}

let args=CommandLine.arguments
if args.count==2 && args[1]=="--tracking-selftest" {
    let expected=[[60.0,60.0],[360,60],[60,200],[360,200]]
    func pattern(_ shift:Int,_ hidden:Int = -1)->(Int,Int,Int)->Bool {
        return {color,x,y in
            if color==hidden {return false}
            let p=expected[color]
            return abs(Double(x)-p[0]-Double(shift))<9 && abs(Double(y)-p[1]+2)<9
        }
    }
    for (color,p) in expected.enumerated() {
        guard let found=locate(p,color,width:450,height:260,matches:pattern(3)),abs(found[0]-p[0]-3)<0.1,abs(found[1]-p[1]+2)<0.1 else {exit(3)}
        guard locate(p,color,width:450,height:260,matches:pattern(12))==nil else {exit(4)}
    }
    guard locate(expected[3],3,width:450,height:260,matches:pattern(3,3))==nil else {exit(5)}
    let large:(Int,Int,Int)->Bool = {color,x,y in
        let p=expected[color]
        return abs(Double(x)-p[0]-3)<=13 && abs(Double(y)-p[1]+2)<=13
    }
    guard locate(expected[2],2,width:450,height:260,searchRadius:24,matches:large) != nil else {exit(6)}
    guard cursorComponent(Array(stride(from:40,through:54,by:2)),span:180)?["x"]==47,
          cursorComponent([40,42,44,46,100,102,104,106],span:180)==nil,
          cursorComponent([40,42,44],span:180)==nil,
          cursorComponent(Array(stride(from:40,through:54,by:2)),span:0)==nil else {exit(7)}
    print("Tracking self-test passed: small translation, enlarged marker, hidden marker, clipped/displaced marker")
    exit(0)
}
guard [4,5,7].contains(args.count) else {exit(2)}
let codedCenterY = args.count >= 5 ? Double(args[4]) : 140
guard let codedCenterY, [140.0,148.0].contains(codedCenterY) else {exit(2)}
let enlargedReference = args.count == 7
let referenceIndex = enlargedReference ? Int(args[6]) : nil
if enlargedReference {
    guard args[5]=="2", codedCenterY==148, let referenceIndex, [0,1].contains(referenceIndex) else {exit(2)}
}
let config=try JSONSerialization.jsonObject(with:Data(contentsOf:URL(fileURLWithPath:args[2]))) as! [[[Double]]]
let asset=AVURLAsset(url:URL(fileURLWithPath:args[1]))
let track=asset.tracks(withMediaType:.video).first!
let transform=track.preferredTransform
let flip=transform.a < 0 && transform.d < 0
guard (transform.a==1 && transform.d==1) || flip else {fputs("Unsupported orientation\n",stderr);exit(2)}
let reader=try AVAssetReader(asset:asset)
let output=AVAssetReaderTrackOutput(track:track,outputSettings:[kCVPixelBufferPixelFormatTypeKey as String:kCVPixelFormatType_32BGRA])
reader.add(output);guard reader.startReading() else {exit(1)}
FileManager.default.createFile(atPath:args[3],contents:nil)
let file=try FileHandle(forWritingTo:URL(fileURLWithPath:args[3]))
// Projective mapping preserves cell centers under camera perspective.
func map(_ c:[[Double]],_ u:Double,_ v:Double)->(Double,Double) {
    let p0=c[0],p1=c[1],p2=c[3],p3=c[2]
    let dx1=p1[0]-p2[0],dx2=p3[0]-p2[0],dx3=p0[0]-p1[0]+p2[0]-p3[0]
    let dy1=p1[1]-p2[1],dy2=p3[1]-p2[1],dy3=p0[1]-p1[1]+p2[1]-p3[1]
    let det=dx1*dy2-dx2*dy1
    let g=(dx3*dy2-dx2*dy3)/det,h=(dx1*dy3-dx3*dy1)/det
    let a=p1[0]-p0[0]+g*p1[0],b=p3[0]-p0[0]+h*p3[0]
    let d=p1[1]-p0[1]+g*p1[1],e=p3[1]-p0[1]+h*p3[1]
    let divisor=g*u+h*v+1
    return ((a*u+b*v+p0[0])/divisor,(d*u+e*v+p0[1])/divisor)
}
var count=0
while let sample=output.copyNextSampleBuffer() {
    guard let buffer=CMSampleBufferGetImageBuffer(sample) else {exit(1)}
    CVPixelBufferLockBaseAddress(buffer,.readOnly)
    let base=CVPixelBufferGetBaseAddress(buffer)!.assumingMemoryBound(to:UInt8.self)
    let width=CVPixelBufferGetWidth(buffer), height=CVPixelBufferGetHeight(buffer), stride=CVPixelBufferGetBytesPerRow(buffer)
    func rgb(_ x:Int,_ y:Int)->(Int,Int,Int) {
        let px=flip ? width-1-x : x,py=flip ? height-1-y : y
        let offset=py*stride+px*4
        return (Int(base[offset+2]),Int(base[offset+1]),Int(base[offset]))
    }
    func matches(_ color:Int,_ x:Int,_ y:Int)->Bool {
        let (r,g,b)=rgb(x,y)
        switch color {
        case 0:return r>110 && b>110 && r>g+40 && b>g+40
        // Cyan includes balanced green/blue. Requiring blue dominance cuts
        // a variable edge off the marker as camera/display color shifts.
        case 1:return g>110 && b>110 && g>r+20 && b>r+20 && abs(b-g)<80
        case 2:return r>110 && r>g+60 && r>b+45
        default:return g>110 && g>r+20 && (g>b+10 || (r>80 && b>140 && g>r+35))
        }
    }
    var patches=[[[Double]]](),trackedCorners=[[[Double]]](),candidateCounts=[Int](),cursorScores=[[Double]](),cursorPositions=[Any]()
    var cursorEvents=[[[String:Any]]]()
    for (patchIndex,original) in config.enumerated() {
        // The fixture markers are 20 logical pixels wide with a 364-pixel
        // center spacing. Scale the search area, not the allowed translation.
        let span=hypot(original[1][0]-original[0][0],original[1][1]-original[0][1])
        let radius=max(16,min(40,Int(ceil(span * 10 / 364))+9))
        let found=original.enumerated().compactMap {locate($0.element,$0.offset,width:width,height:height,searchRadius:radius,matches:matches)}
        candidateCounts.append(found.count)
        guard found.count==4 else {
            patches.append(Array(repeating:Array(repeating:0.0,count:16),count:2));trackedCorners.append([]);cursorScores.append([]);cursorPositions.append(NSNull());cursorEvents.append([]);continue
        }
        let dx=zip(found,original).map{$0[0]-$1[0]}.sorted()[2]
        let dy=zip(found,original).map{$0[1]-$1[1]}.sorted()[2]
        guard hypot(dx,dy)<=6,zip(found,original).allSatisfy({hypot($0[0]-$1[0]-dx,$0[1]-$1[1]-dy)<=3}) else {
            patches.append(Array(repeating:Array(repeating:0.0,count:16),count:2));trackedCorners.append([]);cursorScores.append([]);cursorPositions.append(NSNull());cursorEvents.append([]);continue
        }
        let corners=original.map {[$0[0]+dx,$0[1]+dy]};trackedCorners.append(corners)
        // Optional pointer phase occupies a reserved black region between the
        // timing cells and top fiducials. Yellow cannot match those fiducials.
        var lanes=[Double]()
        for lane in [70,130,250,310] {
            let point=map(corners,Double(lane-10)/364,10.0/140)
            let x=Int(point.0.rounded()),y=Int(point.1.rounded())
            var yellow=0
            for oy in -1...1 { for ox in -1...1 {
                let px=flip ? width-1-x-ox : x+ox,py=flip ? height-1-y-oy : y+oy
                guard px>=0 && px<width && py>=0 && py<height else {exit(2)}
                let offset=py*stride+px*4
                let r=Int(base[offset+2]),g=Int(base[offset+1]),b=Int(base[offset])
                if r>110 && g>110 && min(r,g)>b+40 {yellow+=1}
            }}
            lanes.append(Double(yellow)/9)
        }
        cursorScores.append(lanes)
        let badgeScale = referenceIndex == patchIndex ? 2.0 : 1.0
        let badgeCenterY = referenceIndex == patchIndex ? 164.0 : codedCenterY
        var events=[[String:Any]]()
        for lane in [70,130,250,310] {
            func sample(_ dx:Int,_ dy:Int)->[String:Any] {
            var border=0,borderRGB=[[Int]]()
            for x in [Double(lane)-22*badgeScale,Double(lane)+22*badgeScale] {
                for y in [badgeCenterY-12*badgeScale,badgeCenterY,badgeCenterY+12*badgeScale] {
                    let point=map(corners,(x-10)/364,(150-y)/140)
                    let px=Int(point.0.rounded())+dx,py=Int(point.1.rounded())+dy
                    guard px>=0 && px<width && py>=0 && py<height else {exit(2)}
                    let (r,g,b)=rgb(px,py);borderRGB.append([r,g,b])
                    if r>110 && g>110 && min(r,g)>b+40 {border+=1}
                }
            }
            var cells=[Double]()
            for bit in 0..<12 {
                let x=Double(lane)+Double(-15+(bit%4)*10)*badgeScale,y=badgeCenterY+Double(-8+(bit/4)*8)*badgeScale
                let point=map(corners,(x-10)/364,(150-y)/140)
                let px=Int(point.0.rounded())+dx,py=Int(point.1.rounded())+dy
                guard px>=0 && px<width && py>=0 && py<height else {exit(2)}
                let (r,g,b)=rgb(px,py)
                cells.append(0.2126*Double(r)+0.7152*Double(g)+0.0722*Double(b))
            }
            return ["border_score":Double(border)/6,"border_rgb":borderRGB,"cells":cells,"sample_offset_camera_px":[dx,dy]]
            }
            var primary=sample(0,0),candidates=[[String:Any]]()
            for dy in -2...2 { for dx in -2...2 {
                if dx==0 && dy==0 {continue}
                let candidate=sample(dx,dy)
                if (candidate["border_score"] as! Double)>=4.0/6 {candidates.append(candidate)}
            }}
            primary["candidates"]=candidates;events.append(primary)
        }
        cursorEvents.append(events)
        // A continuous pointer sweep uses the same reserved yellow band.
        // Report one contiguous component only, in fixture logical coordinates.
        var yellowColumns = [Int]()
        for logicalX in Swift.stride(from: 24, through: 352, by: 2) {
            var yellow = 0
            for logicalY in [137.0, 140.0, 143.0] {
                let point = map(corners, Double(logicalX-10)/364, (150-logicalY)/140)
                let x = Int(point.0.rounded()), y = Int(point.1.rounded())
                guard x>=0 && x<width && y>=0 && y<height else {exit(2)}
                let (r,g,b) = rgb(x,y)
                if r>110 && g>110 && min(r,g)>b+40 {yellow += 1}
            }
            if yellow>=2 {yellowColumns.append(logicalX)}
        }
        if let component=cursorComponent(yellowColumns,span:span) {
            cursorPositions.append(component)
        } else {cursorPositions.append(NSNull())}
        var rows=[[Double]]()
        for row in 0..<2 {
            let v=Double(row==0 ? 94 : 46)/140
            var cells=[Double]()
            for bit in 0..<16 {
                let u=Double(32+20*bit)/364
                let point=map(corners,u,v)
                let x=Int(point.0.rounded()),y=Int(point.1.rounded())
                var values=[Double]()
                for dy in -1...1 {for dx in -1...1 {
                    let px=flip ? width-1-x-dx : x+dx, py=flip ? height-1-y-dy : y+dy
                    guard px>=0 && px<width && py>=0 && py<height else {exit(2)}
                    let offset=py*stride+px*4
                    values.append(0.2126*Double(base[offset+2]) + 0.7152*Double(base[offset+1]) + 0.0722*Double(base[offset]))
                }}
                cells.append(values.sorted()[4])
            }
            rows.append(cells)
        }
        patches.append(rows)
    }
    CVPixelBufferUnlockBaseAddress(buffer,.readOnly)
    let record:[String:Any]=["sample":count,"playback_s":CMTimeGetSeconds(CMSampleBufferGetPresentationTimeStamp(sample)),"patches":patches,"tracked_corners":trackedCorners,"tracking_candidate_counts":candidateCounts,"cursor_scores":cursorScores,"cursor_positions":cursorPositions,"cursor_events":cursorEvents]
    var data=try JSONSerialization.data(withJSONObject:record);data.append(10);try file.write(contentsOf:data)
    count+=1
}
guard reader.status == .completed else {exit(1)}
try file.close();print("Read \(count) camera frames")
