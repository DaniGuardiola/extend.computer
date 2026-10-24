"""Actual decoded frame timing, using a generated long-GOP video fixture."""
import json
import shutil
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path


@unittest.skipUnless(sys.platform=='darwin' and shutil.which('swiftc'),'macOS media APIs required')
class ExactExtractTests(unittest.TestCase):
    def test_extract_keeps_requested_time_and_rejects_invalid_times(self):
        with tempfile.TemporaryDirectory() as directory:
            root=Path(directory)
            fixture=root/'writer.swift'
            fixture.write_text('''import Foundation
import AVFoundation
import CoreVideo
let writer=try AVAssetWriter(outputURL:URL(fileURLWithPath:CommandLine.arguments[1]),fileType:.mp4)
let input=AVAssetWriterInput(mediaType:.video,outputSettings:[AVVideoCodecKey:AVVideoCodecType.h264,AVVideoWidthKey:64,AVVideoHeightKey:64,
 AVVideoCompressionPropertiesKey:[AVVideoMaxKeyFrameIntervalKey:90]])
let adaptor=AVAssetWriterInputPixelBufferAdaptor(assetWriterInput:input,sourcePixelBufferAttributes:[kCVPixelBufferPixelFormatTypeKey as String:kCVPixelFormatType_32BGRA,kCVPixelBufferWidthKey as String:64,kCVPixelBufferHeightKey as String:64])
writer.add(input);precondition(writer.startWriting());writer.startSession(atSourceTime:.zero)
for frame in 0..<90 {
 while !input.isReadyForMoreMediaData {Thread.sleep(forTimeInterval:0.001)}
 var buffer:CVPixelBuffer?
 precondition(CVPixelBufferCreate(nil,64,64,kCVPixelFormatType_32BGRA,nil,&buffer)==kCVReturnSuccess)
 CVPixelBufferLockBaseAddress(buffer!,[])
 let bytes=CVPixelBufferGetBaseAddress(buffer!)!.assumingMemoryBound(to:UInt8.self)
 let stride=CVPixelBufferGetBytesPerRow(buffer!)
 for y in 0..<64 {for x in 0..<64 {
  let i=y*stride+x*4
  bytes[i]=frame<30 ? 255 : 0;bytes[i+1]=0;bytes[i+2]=frame>=30 ? 255 : 0;bytes[i+3]=255
 }}
 CVPixelBufferUnlockBaseAddress(buffer!,[])
 precondition(adaptor.append(buffer!,withPresentationTime:CMTime(value:Int64(frame),timescale:30)))
}
input.markAsFinished();let finished=DispatchSemaphore(value:0)
writer.finishWriting {finished.signal()};finished.wait();precondition(writer.status == .completed)
''')
            source=Path(__file__).parent/'android-camera/extract.swift'
            extractor=root/'extract';writer=root/'writer';video=root/'fixture.mp4';image=root/'frame.png'
            for code,binary in [(source,extractor),(fixture,writer)]:
                subprocess.run(['swiftc','-O',str(code),'-o',str(binary)],check=True,capture_output=True)
            subprocess.run([str(writer),str(video)],check=True,capture_output=True,timeout=15)
            subprocess.run([str(extractor),str(video),str(image),'1.47'],check=True,capture_output=True)
            timing=json.loads(Path(str(image)+'.timing.json').read_text())
            self.assertEqual(timing['selection'],'zero-tolerance-v1')
            self.assertLessEqual(abs(timing['actual_playback_s']-1.47),1/30+.001)
            self.assertGreater(timing['actual_playback_s'],1)
            self.assertTrue(image.is_file())
            for invalid in ['nan','-1','100']:
                result=subprocess.run([str(extractor),str(video),str(root/'invalid.png'),invalid],capture_output=True)
                self.assertNotEqual(result.returncode,0)
            self.assertFalse((root/'invalid.png').exists())

if __name__=='__main__':unittest.main()
