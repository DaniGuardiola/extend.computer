import subprocess, tempfile, unittest
from pathlib import Path
from capture_phone import validate_orientation, camera_orientation, wait_receipt

class OrientationTests(unittest.TestCase):
    def test_provenance_rejects_stale_hint_or_portrait(self):
        receipt={'orientation_policy':'attached-landscape-stable-v1','sensor_orientation':90,'display_rotation':1,'orientation_hint':0,'window_bounds':[0,0,2400,1080],'orientation_stable_ms':250}
        self.assertEqual(validate_orientation(receipt)['status'],'orientation_acquired')
        for changed in [{'orientation_hint':90},{'window_bounds':[0,0,1080,2400]},{'display_rotation':True},{'orientation_stable_ms':249}]:
            with self.subTest(changed=changed),self.assertRaises(ValueError):validate_orientation({**receipt,**changed})
        self.assertEqual(validate_orientation({**receipt,'display_rotation':3,'orientation_hint':180,'requested_orientation':'reverse-landscape'})['orientation_hint'],180)
        with self.assertRaises(ValueError):validate_orientation({**receipt,'requested_orientation':'reverse-landscape'})
        with self.assertRaises(ValueError):camera_orientation('portrait')
        self.assertEqual(validate_orientation({})['status'],'unverified_legacy_orientation')

    def test_unsettled_orientation_cannot_extend_readiness_deadline(self):
        import subprocess
        times=iter([0, 1, 2])
        def absent(_):raise subprocess.CalledProcessError(1,['adb'])
        with tempfile.TemporaryDirectory() as root:
            with self.assertRaises(TimeoutError):
                wait_receipt(absent,'ready.json',2,Path(root),clock=lambda:next(times),sleep=lambda _:None)

    def test_gate_waits_for_landscape_and_restarts_on_rotation_or_resize(self):
        source=Path(__file__).parent/'android-camera/src/computer/extend/benchmarkcamera/OrientationGate.java'
        java=Path('/opt/homebrew/opt/openjdk/bin')
        with tempfile.TemporaryDirectory() as root:
            harness=Path(root)/'GateTest.java'
            harness.write_text("""package computer.extend.benchmarkcamera;
public class GateTest {
 static void check(boolean value) { if(!value) throw new AssertionError(); }
 public static void main(String[] args) {
  check(OrientationGate.matchesRequest("reverse-landscape",3));
  check(!OrientationGate.matchesRequest("reverse-landscape",1));
  check(!OrientationGate.matchesRequest("portrait",1));
  check(OrientationGate.supportsFocusRegions(0)); check(OrientationGate.supportsFocusRegions(180));
  check(!OrientationGate.supportsFocusRegions(90)); check(!OrientationGate.supportsFocusRegions(270));
  OrientationGate g=new OrientationGate();
  check(!g.observe(0,1080,2400,0)); check(!g.observe(1,2400,1080,100));
  check(!g.observe(1,2400,1080,349)); check(g.observe(1,2400,1080,350));
  check(!g.observe(3,2400,1080,400)); check(g.observe(3,2400,1080,650));
  check(!g.observe(3,2300,1080,700)); check(!g.observe(3,2300,1080,949));
  check(g.observe(3,2300,1080,950)); check(!g.observe(3,1080,2400,1000));
  check(!g.observe(3,2300,1080,1300));
  check(OrientationGate.hint(90,1)==0); check(OrientationGate.hint(90,3)==180);
 }
}""")
            subprocess.run([str(java/'javac'),'-d',root,str(source),str(harness)],check=True)
            subprocess.run([str(java/'java'),'-cp',root,'computer.extend.benchmarkcamera.GateTest'],check=True)
