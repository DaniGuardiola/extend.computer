import hashlib, tempfile, unittest, zipfile
from pathlib import Path
from unittest.mock import patch
from bootstrap import safe_zip, fetch
from bench import select_phone, repository

class SetupTests(unittest.TestCase):
    def test_duplicate_transports_one_phone(self):
        def query(argv):
            if argv[-1]=='devices':return 'List of devices attached\n10.0.0.1:1111\tdevice\nadb-unit._adb-tls-connect._tcp\tdevice\n'
            return 'same-physical-phone'
        with patch('bench.read',side_effect=query):self.assertEqual(select_phone('adb'),'adb-unit._adb-tls-connect._tcp')
    def test_two_phones_require_selection(self):
        def query(argv):
            if argv[-1]=='devices':return 'List of devices attached\na\tdevice\nb\tdevice\n'
            return argv[2]
        with patch('bench.read',side_effect=query),self.assertRaises(RuntimeError):select_phone('adb')
    def test_offline_remembered_phone_not_replaced(self):
        with patch('bench.read',side_effect=['List of devices attached\na\tdevice','other']),self.assertRaises(RuntimeError):select_phone('adb',remembered='wrong')
    def test_unauthorized_not_selected(self):
        with patch('bench.read',return_value='List of devices attached\na\tunauthorized'),self.assertRaises(RuntimeError):select_phone('adb')
    def test_archive_traversal_rejected(self):
        with tempfile.TemporaryDirectory() as root:
            archive=Path(root)/'bad.zip'
            with zipfile.ZipFile(archive,'w') as z:z.writestr('../escape','bad')
            with self.assertRaises(ValueError):safe_zip(archive,Path(root)/'sdk')
    def test_sdk_symlink_rejected(self):
        with tempfile.TemporaryDirectory() as root:
            archive=Path(root)/'bad.zip'
            info=zipfile.ZipInfo('link');info.external_attr=0o120777<<16
            with zipfile.ZipFile(archive,'w') as z:z.writestr(info,'/outside')
            with self.assertRaises(ValueError):safe_zip(archive,Path(root)/'sdk')
    def test_cached_verified_download_offline(self):
        with tempfile.TemporaryDirectory() as root:
            path=Path(root)/'package';path.write_bytes(b'verified')
            with patch('bootstrap.urllib.request.urlopen',side_effect=AssertionError('Network not needed')):
                self.assertEqual(fetch('https://example.invalid/package',path,hashlib.sha256(b'verified').hexdigest()),path)
    def test_plain_http_rejected(self):
        with self.assertRaises(ValueError):fetch('http://example.invalid/x',Path('/tmp/unused'))
    def test_missing_public_core(self):
        with tempfile.TemporaryDirectory() as root,self.assertRaises(RuntimeError):repository(root,{})
