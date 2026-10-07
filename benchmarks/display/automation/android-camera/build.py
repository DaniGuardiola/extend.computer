import argparse,os,subprocess,zipfile
from pathlib import Path
root=Path(__file__).resolve().parent
parser=argparse.ArgumentParser()
parser.add_argument('--sdk',type=Path,required=True)
parser.add_argument('--java-bin',type=Path,required=True)
args=parser.parse_args()
sdk=args.sdk;java=args.java_bin
tools=sdk/'build-tools/35.0.0' if (sdk/'build-tools/35.0.0').is_dir() else sdk/'android-15'
jar=sdk/'platforms/android-35/android.jar' if (sdk/'platforms/android-35/android.jar').is_file() else sdk/'android-35/android.jar'
if not jar.is_file() or not (tools/'lib/d8.jar').is_file():raise SystemExit('SDK needs Android platform 35 and build-tools 35.0.0')
import shutil
for directory in ['classes','dex']:
 shutil.rmtree(root/directory,ignore_errors=True);(root/directory).mkdir()
def run(args):subprocess.run([str(x) for x in args],check=True)
for executable in ['aapt2','zipalign']:os.chmod(tools/executable,0o755)
run([java/'javac','-source','8','-target','8','-classpath',jar,'-d',root/'classes',*root.glob('src/**/*.java')])
run([java/'java','-cp',tools/'lib/d8.jar','com.android.tools.r8.D8','--min-api','23','--lib',jar,'--output',root/'dex',*root.glob('classes/**/*.class')])
run([tools/'aapt2','link','-I',jar,'--manifest',root/'AndroidManifest.xml','-o',root/'unsigned.apk'])
with zipfile.ZipFile(root/'unsigned.apk','a') as z:z.write(root/'dex/classes.dex','classes.dex')
run([tools/'zipalign','-f','4',root/'unsigned.apk',root/'aligned.apk'])
if not (root/'debug.keystore').exists():
 run([java/'keytool','-genkeypair','-keystore',root/'debug.keystore','-storepass','android','-keypass','android','-alias','benchmark','-keyalg','RSA','-validity','3650','-dname','CN=Local Benchmark Test'])
run([java/'java','-jar',tools/'lib/apksigner.jar','sign','--ks',root/'debug.keystore','--ks-key-alias','benchmark','--ks-pass','pass:android','--key-pass','pass:android','--out',root/'benchmark-camera.apk',root/'aligned.apk'])
print('Built benchmark-camera.apk')
