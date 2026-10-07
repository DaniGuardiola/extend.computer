import copy
import hashlib
import json
from pathlib import Path
import tempfile
import unittest
from benchmarks.display.core import analyze, compare, load_run, validate_manifest, validate_record


def manifest():
    return {"schema_version":1,"run_id":"synthetic-test","product":{"name":"synthetic","version":"1","build_sha256":"a"*64},"adapter":{"name":"replay","version":"1"},"environment":{"source":"host-a-os-1","receiver":"host-b-os-1","network":"direct-lan","renderer":"wkwebview-v1","idle":True},"profile":{"logical_width":1728,"logical_height":1085,"pixel_width":1728,"pixel_height":1085,"scale":1,"fps":60,"bitrate_mbps":20,"mode":"extend","codec":"h264"},"workload":{"version":"synthetic-v1","sha256":"b"*64,"duration_s":1,"scenes":["motion"],"geometry":[1728,1085,1,1525,953]},"instrumentation":{"clock_domains":["receiver","source"],"observer_effect":"not calibrated","capabilities":["received","decoded","gpu_complete","layer_submit"]},"artifacts":{}}


def phase(clock="receiver"):
    return {"kind":"phase","run_id":"synthetic-test","phase":"motion","clock":clock,"at_ns":0,"end_ns":1000000000,"geometry":[1728,1085,1,1525,953],"hidden":False,"resized":False,"cancelled":False,"route_changed":False,"disconnected":False,"source_hz":60}


def frame(stage="gpu_complete",clock="receiver",at=1000000,id="one"):
    return {"kind":"frame","run_id":"synthetic-test","phase":"motion","clock":clock,"at_ns":at,"stage":stage,"frame_id":id,"bytes":0}


class CoreTests(unittest.TestCase):
    def test_missing_required_scene_invalidates_campaign(self):
        m=manifest();m["workload"]["scenes"].append("scroll")
        report=analyze(m,[phase(),frame()])
        self.assertFalse(report["phases"]["scroll/missing"]["valid"])

    def test_duplicate_frames_not_duplicate_delivery(self):
        result=analyze(manifest(),[phase(),frame(),frame()])
        self.assertEqual(result["phases"]["motion/receiver"]["metrics"][0]["value"],1)

    def test_same_frame_latency_and_no_cross_clock_join(self):
        result=analyze(manifest(),[phase(),frame("received",at=1000000),frame("decoded",at=3000000),frame("gpu_complete",clock="source",at=5000000)])
        metrics=result["phases"]["motion/receiver"]["metrics"]
        timing=[r for r in metrics if r["metric"]=="stage_latency"]
        self.assertEqual(len(timing),1);self.assertEqual(timing[0]["value"]["median"],2)

    def test_both_host_windows_supported(self):
        result=analyze(manifest(),[phase(),phase("source"),frame(),frame(clock="source")])
        self.assertEqual(len(result["phases"]),2)

    def test_invalid_phases_produce_no_metrics(self):
        for flag in ["hidden","resized","cancelled","route_changed","disconnected"]:
            p=phase();p[flag]=True
            row=analyze(manifest(),[p,frame()])["phases"]["motion/receiver"]
            self.assertFalse(row["valid"]);self.assertEqual(row["metrics"],[])

    def test_duration_geometry_and_low_source_cadence_rejected(self):
        for key,value in [("end_ns",3000000000),("geometry",[1,1,1,1,1]),("source_hz",10)]:
            p=phase();p[key]=value
            self.assertFalse(analyze(manifest(),[p])["phases"]["motion/receiver"]["valid"])

    def test_counters_at_different_stages_never_compare(self):
        left=analyze(manifest(),[phase(),frame()]);right=analyze(manifest(),[phase(),frame("layer_submit")])
        self.assertEqual(compare(left,right)["rows"],[])
        self.assertTrue(compare(left,right)["unavailable"])

    def test_context_mismatch_blocks_comparison(self):
        left=analyze(manifest(),[phase(),frame()]);m=manifest();m["environment"]["receiver"]="different-host"
        right=analyze(m,[phase(),frame()]);self.assertFalse(compare(left,right)["comparable"])
        self.assertEqual(compare(left,right)["rows"],[])

    def test_codec_difference_is_visible(self):
        left=analyze(manifest(),[phase(),frame()]);m=manifest();m["profile"]["codec"]="hevc"
        report=compare(left,analyze(m,[phase(),frame()]))
        self.assertTrue(report["comparable"]);self.assertEqual(report["codec_context"],["h264","hevc"])

    def test_unknown_fields_and_nan_fail_closed(self):
        m=manifest();r=frame();r["clipboard"]="secret"
        with self.assertRaises(ValueError):validate_record(r,m)
        m["profile"]["fps"]=float("nan")
        with self.assertRaises(ValueError):validate_manifest(m)

    def test_artifact_tampering_rejected(self):
        with tempfile.TemporaryDirectory() as d:
            root=Path(d);events=json.dumps(phase())+"\n";m=manifest();m["artifacts"]={"events.jsonl":hashlib.sha256(events.encode()).hexdigest()}
            (root/"manifest.json").write_text(json.dumps(m));(root/"events.jsonl").write_text(events)
            self.assertIn("motion/receiver",load_run(root)["phases"])
            (root/"events.jsonl").write_text(events+" ")
            with self.assertRaises(ValueError):load_run(root)

    def test_artifact_traversal_rejected(self):
        m=manifest();m["artifacts"]={"../outside":"a"*64}
        with self.assertRaises(ValueError):validate_manifest(m)

if __name__=="__main__":unittest.main()
