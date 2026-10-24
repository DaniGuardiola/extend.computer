import json,sys,tempfile,unittest
from pathlib import Path
sys.path.insert(0,str(Path(__file__).resolve().parents[1]/'campaign'))
from run import prepare_reanalysis_copy,digest,artifact_inventory


class ReanalysisTests(unittest.TestCase):
    def fixture(self,root):
        source=root/'original';camera=source/'01-motion'/'camera';(camera/'raw').mkdir(parents=True)
        (camera/'raw/video.mp4').write_bytes(b'original captured footage')
        (camera/'summary.json').write_text(json.dumps({'video_sha256':digest(camera/'raw/video.mp4')}))
        (camera/'provenance.json').write_text('{"old": true}')
        (camera.parent/'source.json').write_text('{}')
        report={'status':'complete_with_declared_limits','settings':{'campaign_profile':'diagnostic','scene_selection':['motion']},
                'limits':[],'phases':[{'scene':'motion','repetition':1,'status':'measured',
                'route':{'classification':'direct'},'optical':{'old':True},'analysis_pipeline':{'old':True}}]}
        (source/'report.json').write_text(json.dumps(report))
        (source/'manifest.json').write_text(json.dumps({'artifacts':artifact_inventory(source)}))
        return source

    def test_reanalysis_preserves_original_and_media_with_explicit_provenance(self):
        with tempfile.TemporaryDirectory() as directory:
            root=Path(directory);source=self.fixture(root)
            before={str(p.relative_to(source)):p.read_bytes() for p in source.rglob('*') if p.is_file()}
            target=prepare_reanalysis_copy(source,root/'new-analysis')
            report=json.loads((target/'report.json').read_text())
            self.assertEqual(report['phases'][0]['status'],'captured')
            self.assertNotIn('optical',report['phases'][0]);self.assertNotIn('analysis_pipeline',report['phases'][0])
            self.assertEqual(report['phases'][0]['route'],{'classification':'direct'})
            archived=json.loads((target/report['offline_reanalysis']['previous_analysis']/'checkpoint.json').read_text())
            self.assertEqual(archived['phases'][0]['optical'],{'old':True})
            self.assertEqual(digest(target/'01-motion/camera/raw/video.mp4'),digest(source/'01-motion/camera/raw/video.mp4'))
            (target/'01-motion/camera/provenance.json').write_text('{}')
            self.assertEqual(before,{str(p.relative_to(source)):p.read_bytes() for p in source.rglob('*') if p.is_file()})

    def test_rejects_corrupt_footage_before_creating_output(self):
        with tempfile.TemporaryDirectory() as directory:
            root=Path(directory);source=self.fixture(root)
            (source/'01-motion/camera/raw/video.mp4').write_bytes(b'changed')
            with self.assertRaises(RuntimeError):prepare_reanalysis_copy(source,root/'new')
            self.assertFalse((root/'new').exists())

    def test_rejects_overwrite_nested_output_and_incomplete_campaign(self):
        with tempfile.TemporaryDirectory() as directory:
            root=Path(directory);source=self.fixture(root)
            for target in [source,source/'nested',root]:
                with self.assertRaises(ValueError):prepare_reanalysis_copy(source,target)
            (root/'exists').mkdir()
            with self.assertRaises(ValueError):prepare_reanalysis_copy(source,root/'exists')
            report=json.loads((source/'report.json').read_text());report['status']='failed'
            (source/'report.json').write_text(json.dumps(report))
            with self.assertRaises(ValueError):prepare_reanalysis_copy(source,root/'new')
            self.assertFalse((root/'new').exists())

if __name__=='__main__':unittest.main()
