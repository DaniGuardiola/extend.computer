import Foundation
import AVFoundation
import CoreVideo
func locate(_ expected:[Double],_ color:Int,width:Int,height:Int,matches:(Int,Int,Int)->Bool)->[Double]? {
    let cx=Int(expected[0].rounded()),cy=Int(expected[1].rounded()),radius=16,side=33
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
        if points.count<25 || points.count>700 {continue}
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
    print("Tracking self-test passed: small translation, hidden marker, clipped/displaced marker")
    exit(0)
}
guard args.count==4 else {exit(2)}
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
    var patches=[[[Double]]](),trackedCorners=[[[Double]]](),candidateCounts=[Int]()
    for original in config {
        let found=original.enumerated().compactMap {locate($0.element,$0.offset,width:width,height:height,matches:matches)}
        candidateCounts.append(found.count)
        guard found.count==4 else {
            patches.append(Array(repeating:Array(repeating:0.0,count:16),count:2));trackedCorners.append([]);continue
        }
        let dx=zip(found,original).map{$0[0]-$1[0]}.sorted()[2]
        let dy=zip(found,original).map{$0[1]-$1[1]}.sorted()[2]
        guard hypot(dx,dy)<=6,zip(found,original).allSatisfy({hypot($0[0]-$1[0]-dx,$0[1]-$1[1]-dy)<=3}) else {
            patches.append(Array(repeating:Array(repeating:0.0,count:16),count:2));trackedCorners.append([]);continue
        }
        let corners=original.map {[$0[0]+dx,$0[1]+dy]};trackedCorners.append(corners)
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
    let record:[String:Any]=["sample":count,"playback_s":CMTimeGetSeconds(CMSampleBufferGetPresentationTimeStamp(sample)),"patches":patches,"tracked_corners":trackedCorners,"tracking_candidate_counts":candidateCounts]
    var data=try JSONSerialization.data(withJSONObject:record);data.append(10);try file.write(contentsOf:data)
    count+=1
}
guard reader.status == .completed else {exit(1)}
try file.close();print("Read \(count) camera frames")
