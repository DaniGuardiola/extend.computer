"""Workload provenance tests use synthetic files; no process or camera starts."""
import hashlib
import json
from pathlib import Path
import tempfile
import unittest
from camera_run import source_workload_provenance

class WorkloadProvenanceTests(unittest.TestCase):
    def setUp(self):
        self.temp=tempfile.TemporaryDirectory();self.addCleanup(self.temp.cleanup)
        self.repo=Path(self.temp.name);self.build=self.repo/'build';self.build.mkdir()
        self.source=self.repo/'benchmarks/display/workloads/campaign-workload.swift'
        self.source.parent.mkdir(parents=True);self.source.write_text('synthetic source')
        self.binary=self.build/'campaign-workload';self.binary.write_bytes(b'synthetic binary')
        self.record=self.build/'campaign-workload.build.json'
        self.receipt={'source_sha256':self.sha(self.source),'binary_sha256':self.sha(self.binary),'compiler':'synthetic'}
        self.record.write_text(json.dumps(self.receipt))
    @staticmethod
    def sha(path):return hashlib.sha256(path.read_bytes()).hexdigest()
    def test_direct_command_snapshots_actual_source_binary_and_receipt(self):
        result=source_workload_provenance([str(self.binary),'2','motion'],self.repo,self.build)
        self.assertEqual(result['status'],'verified_direct_local')
        self.assertEqual(result['build_receipt_sha256'],self.sha(self.record))
        self.assertEqual(result['build'],self.receipt)
        self.binary.write_bytes(b'later replacement')
        self.assertEqual(result['binary_sha256'],self.receipt['binary_sha256'])
    def test_wrapped_or_missing_command_remains_unknown(self):
        for command in (None,['python3','client.py',str(self.binary)]):
            self.assertEqual(source_workload_provenance(command,self.repo,self.build)['status'],'unknown')
    def test_stale_source_or_binary_fails_before_capture(self):
        self.source.write_text('changed source')
        with self.assertRaisesRegex(ValueError,'before capture'):
            source_workload_provenance([str(self.binary)],self.repo,self.build)
        self.source.write_text('synthetic source');self.binary.write_bytes(b'changed binary')
        with self.assertRaisesRegex(ValueError,'before capture'):
            source_workload_provenance([str(self.binary)],self.repo,self.build)

if __name__=='__main__':unittest.main()
