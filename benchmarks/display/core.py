"""Strict normalized benchmark format. No device control or third-party adapters."""
import hashlib
import json
import math
import statistics
from pathlib import Path

VERSION = 1
SCENES = {"static", "scroll", "panel", "motion"}
STAGES = {"capture_complete", "encoded", "received", "decoded", "gpu_complete", "layer_submit"}
METRICS = {
    "source_cpu": ("percent_one_core", "process_cpu_time"),
    "receiver_cpu": ("percent_one_core", "process_cpu_time"),
    "source_memory": ("bytes", "resident_memory"),
    "receiver_memory": ("bytes", "resident_memory"),
    "wire_bitrate": ("mbps", "connection_bytes"),
    "payload_bitrate": ("mbps", "encoded_payload"),
    "network_rtt": ("ms", "transport_rtt_estimate"),
    "physical_latency": ("ms", "camera_source_to_receiver"),
    "psnr": ("db", "registered_image_psnr"),
    "ssim": ("ratio", "registered_image_ssim"),
}


def exact(value, keys):
    if not isinstance(value, dict) or set(value) != set(keys):
        raise ValueError("Unexpected or missing contract fields")


def number(value, positive=False):
    if isinstance(value, bool) or not isinstance(value, (int, float)) or not math.isfinite(value) or value < 0 or (positive and value == 0):
        raise ValueError("Invalid finite numeric field")
    return value


def label(value):
    if not isinstance(value, str) or not value or len(value) > 160 or any(ord(c) < 32 for c in value):
        raise ValueError("Invalid metadata label")
    return value


def validate_manifest(m):
    exact(m, {"schema_version", "run_id", "product", "adapter", "environment", "profile", "workload", "instrumentation", "artifacts"})
    if type(m["schema_version"]) is not int or m["schema_version"] != VERSION:
        raise ValueError("Unsupported manifest version")
    label(m["run_id"])
    exact(m["product"], {"name", "version", "build_sha256"})
    exact(m["adapter"], {"name", "version"})
    for obj in (m["product"], m["adapter"]):
        for value in obj.values(): label(value)
    for obj, key in ((m["product"], "build_sha256"), (m["workload"], "sha256")):
        if not isinstance(obj.get(key), str) or len(obj[key]) != 64 or any(c not in "0123456789abcdef" for c in obj[key]):
            raise ValueError("Expected SHA-256")
    exact(m["environment"], {"source", "receiver", "network", "renderer", "idle"})
    for key in ("source", "receiver", "network", "renderer"): label(m["environment"][key])
    if not isinstance(m["environment"]["idle"], bool): raise ValueError("Invalid idle flag")
    p=m["profile"]
    exact(p, {"logical_width", "logical_height", "pixel_width", "pixel_height", "scale", "fps", "bitrate_mbps", "mode", "codec"})
    for key in ("logical_width", "logical_height", "pixel_width", "pixel_height", "scale", "fps", "bitrate_mbps"): number(p[key], True)
    if p["mode"] not in {"extend", "mirror"}: raise ValueError("Unknown display mode")
    label(p["codec"])
    w=m["workload"]
    exact(w, {"version", "sha256", "duration_s", "geometry", "scenes"})
    label(w["version"]);number(w["duration_s"], True)
    if not isinstance(w["scenes"],list) or not w["scenes"] or len(set(w["scenes"]))!=len(w["scenes"]) or not all(scene in SCENES for scene in w["scenes"]): raise ValueError("Expected distinct measured scenes")
    if not isinstance(w["geometry"], list) or len(w["geometry"]) != 5: raise ValueError("Expected viewport width, height, scale, canvas width, height")
    for value in w["geometry"]: number(value, True)
    if w["geometry"][:3] != [p["logical_width"],p["logical_height"],p["scale"]]: raise ValueError("Profile/viewport mismatch")
    if p["pixel_width"] != p["logical_width"]*p["scale"] or p["pixel_height"] != p["logical_height"]*p["scale"]: raise ValueError("Profile pixel geometry mismatch")
    exact(m["instrumentation"], {"clock_domains", "observer_effect", "capabilities"})
    i=m["instrumentation"]
    if not isinstance(i["clock_domains"], list) or not i["clock_domains"]: raise ValueError("Clock domains required")
    for domain in i["clock_domains"]: label(domain)
    if len(set(i["clock_domains"])) != len(i["clock_domains"]): raise ValueError("Duplicate clock domains")
    label(i["observer_effect"])
    if not isinstance(i["capabilities"], list) or not all(c in METRICS or c in STAGES for c in i["capabilities"]): raise ValueError("Unknown capability")
    if not isinstance(m["artifacts"], dict): raise ValueError("Artifact inventory required")
    for name, digest in m["artifacts"].items():
        path=Path(name)
        if path.is_absolute() or ".." in path.parts or not name: raise ValueError("Artifact must remain inside run directory")
        if not isinstance(digest,str) or len(digest)!=64 or any(c not in "0123456789abcdef" for c in digest): raise ValueError("Invalid artifact digest")
    return m


def validate_record(r, m):
    kind=r.get("kind") if isinstance(r,dict) else None
    base={"kind", "run_id", "phase", "clock", "at_ns"}
    fields={
        "phase": {"end_ns", "geometry", "hidden", "resized", "cancelled", "route_changed", "disconnected", "source_hz"},
        "frame": {"stage", "frame_id", "bytes"},
        "sample": {"metric", "value", "unit", "definition", "scope"},
    }
    if kind not in fields: raise ValueError("Unknown record kind")
    exact(r,base|fields[kind]);label(r["run_id"])
    if r["run_id"] != m["run_id"] or r["phase"] not in m["workload"]["scenes"] or r["clock"] not in m["instrumentation"]["clock_domains"]: raise ValueError("Record identity, phase, or clock mismatch")
    for key in ("at_ns",):
        number(r[key])
        if not isinstance(r[key],int): raise ValueError("Timestamp must be integer nanoseconds")
    if kind=="phase":
        number(r["end_ns"])
        if not isinstance(r["end_ns"],int) or r["end_ns"]<=r["at_ns"]: raise ValueError("Invalid phase interval")
        if not isinstance(r["geometry"],list) or len(r["geometry"])!=5: raise ValueError("Invalid phase geometry")
        for v in r["geometry"]: number(v, True)
        for key in ("hidden", "resized", "cancelled", "route_changed", "disconnected"):
            if not isinstance(r[key],bool): raise ValueError("Invalid phase flag")
        if r["source_hz"] is not None: number(r["source_hz"])
    elif kind=="frame":
        if r["stage"] not in STAGES or r["stage"] not in m["instrumentation"]["capabilities"]: raise ValueError("Undeclared frame capability")
        label(r["frame_id"]);number(r["bytes"])
        if not isinstance(r["bytes"],int): raise ValueError("Byte count must be integer")
    else:
        if r["metric"] not in METRICS or r["metric"] not in m["instrumentation"]["capabilities"]: raise ValueError("Undeclared metric capability")
        if (r["unit"],r["definition"]) != METRICS[r["metric"]]: raise ValueError("Metric definition mismatch")
        number(r["value"]);label(r["scope"])
        if r["metric"]=="ssim" and r["value"]>1: raise ValueError("SSIM out of range")
    return r


def stats(values):
    values=sorted(values)
    if not values: return None
    return {"count":len(values),"median":statistics.median(values),"p95":values[max(0,math.ceil(.95*len(values))-1)],"max":values[-1]}


def analyze(m, records):
    validate_manifest(m)
    for r in records: validate_record(r,m)
    phases=[r for r in records if r["kind"]=="phase"]
    if len({(r["phase"],r["clock"]) for r in phases})!=len(phases): raise ValueError("Duplicate measured phase")
    for clock in m["instrumentation"]["clock_domains"]:
        windows=sorted((p for p in phases if p["clock"]==clock),key=lambda p:p["at_ns"])
        if any(a["end_ns"]>b["at_ns"] for a,b in zip(windows,windows[1:])): raise ValueError("Overlapping measured phases")
    output={}
    for p in phases:
        scene=p["phase"];duration=(p["end_ns"]-p["at_ns"])/1e9
        reasons=[key for key in ("hidden","resized","cancelled","route_changed","disconnected") if p[key]]
        if p["geometry"]!=m["workload"]["geometry"]: reasons.append("geometry_mismatch")
        if abs(duration-m["workload"]["duration_s"])>max(.5,m["workload"]["duration_s"]*.02): reasons.append("duration_mismatch")
        if not m["environment"]["idle"]: reasons.append("hosts_not_idle")
        if scene!="static" and (p["source_hz"] is None or p["source_hz"]<m["profile"]["fps"]*.95): reasons.append("source_cadence_low")
        # Windows use one host clock. Cross-host analysis requires an explicit
        # calibrated mapping; no wall-clock subtraction or guessed frame IDs.
        selected=[r for r in records if r["kind"]!="phase" and r["phase"]==scene and r["clock"]==p["clock"] and p["at_ns"]<=r["at_ns"]<p["end_ns"]]
        frames={}
        for r in selected:
            if r["kind"]=="frame": frames.setdefault(r["stage"],{}).setdefault(r["frame_id"],r)
        if not frames and set(m["instrumentation"]["capabilities"]) & STAGES: reasons.append("missing_frame_observations")
        metrics=[]
        def add(name,unit,definition,scope,value):
            metrics.append({"metric":name,"unit":unit,"definition":definition,"scope":scope,"value":value})
        for stage, rows in frames.items():
            times=sorted(r["at_ns"] for r in rows.values())
            add("fps","frames/s",stage,"unique_frame_ids",len(times)/duration)
            add("frame_gap","ms",stage,"unique_frame_ids",stats([(b-a)/1e6 for a,b in zip(times,times[1:])]))
            if stage=="encoded": add("payload_bitrate","mbps","encoded_payload","video",sum(r["bytes"] for r in rows.values())*8/duration/1e6)
        for first,last in (("received","decoded"),("received","gpu_complete"),("capture_complete","encoded")):
            a=frames.get(first,{});b=frames.get(last,{})
            durations=[(b[k]["at_ns"]-a[k]["at_ns"])/1e6 for k in a.keys()&b.keys() if b[k]["at_ns"]>=a[k]["at_ns"]]
            if durations: add("stage_latency","ms",first+"_to_"+last,"same_host_clock",stats(durations))
        samples={}
        for r in selected:
            if r["kind"]=="sample": samples.setdefault((r["metric"],r["unit"],r["definition"],r["scope"]),[]).append(r["value"])
        for (name,unit,definition,scope),values in samples.items(): add(name,unit,definition,scope,stats(values))
        output[scene+"/"+p["clock"]]={"scene":scene,"valid":not reasons,"invalid_reasons":reasons,"duration_s":duration,"clock":p["clock"],"metrics":metrics if not reasons else []}
    for scene in set(m["workload"]["scenes"])-{p["phase"] for p in phases}:
        output[scene+"/missing"]={"scene":scene,"valid":False,"invalid_reasons":["missing_measured_phase"],"duration_s":0,"clock":None,"metrics":[]}
    return {"schema_version":VERSION,"run_id":m["run_id"],"manifest":m,"phases":output,"limits":["GPU completion is not physical scanout.","Only same-clock stage latency is computed.","Absent metrics remain unavailable; sample percentiles are not packet distributions."]}


def compare(left,right):
    a,b=left["manifest"],right["manifest"]
    problems=[]
    for key in ("source","receiver","network","renderer","idle"):
        if a["environment"][key]!=b["environment"][key]: problems.append("environment."+key)
    for key in a["profile"]:
        if key!="codec" and a["profile"][key]!=b["profile"][key]: problems.append("profile."+key)
    if a["workload"]!=b["workload"]: problems.append("workload")
    rows=[];missing=[]
    for scene in sorted(left["phases"].keys()|right["phases"].keys()):
        x=left["phases"].get(scene);y=right["phases"].get(scene)
        if not x or not y or not x["valid"] or not y["valid"]:
            missing.append({"phase":scene,"reason":"missing_or_invalid_phase"});continue
        def index(p): return {(r["metric"],r["unit"],r["definition"],r["scope"]):r["value"] for r in p["metrics"]}
        ix,iy=index(x),index(y)
        for key in sorted(ix.keys()|iy.keys()):
            if key not in ix or key not in iy:
                missing.append({"phase":scene,"metric":key[0],"definition":key[2],"scope":key[3],"reason":"no_equivalent_measurement"});continue
            if not problems: rows.append({"phase":scene,"metric":key[0],"unit":key[1],"definition":key[2],"scope":key[3],"left":ix[key],"right":iy[key]})
    return {"comparable":not problems,"mismatches":problems,"codec_context":[a["profile"]["codec"],b["profile"]["codec"]],"rows":rows,"unavailable":missing}


def load_run(directory):
    directory=Path(directory)
    m=json.loads((directory/"manifest.json").read_text());validate_manifest(m)
    for name,digest in m["artifacts"].items():
        path=(directory/name).resolve()
        if not path.is_relative_to(directory.resolve()): raise ValueError("Artifact escapes run directory")
        if hashlib.sha256(path.read_bytes()).hexdigest()!=digest: raise ValueError("Artifact digest mismatch")
    if "events.jsonl" not in m["artifacts"]: raise ValueError("Normalized events must be hashed")
    records=[json.loads(line) for line in (directory/"events.jsonl").read_text().splitlines() if line.strip()]
    return analyze(m,records)
