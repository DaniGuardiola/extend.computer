"""Repository-local dependency setup. No global package manager changes."""
import hashlib, json, os, platform, shutil, subprocess, sys, tarfile, urllib.request, zipfile
from pathlib import Path
ROOT=Path(__file__).resolve().parent
CACHE=ROOT/'.cache'
SDK_FILES={
 'platform':('https://dl.google.com/android/repository/platform-35_r02.zip','0bb560a90a7a2cbd0dd8348224d518b638fe7949'),
 'build-tools':('https://dl.google.com/android/repository/build-tools_r35_macosx.zip','93ab8ce91230e067b5add4bfa79919c52b27f072'),
}

def fetch(url,path,digest=None,algorithm='sha256'):
    if not url.startswith('https://'):raise ValueError('HTTPS download required')
    path.parent.mkdir(parents=True,exist_ok=True)
    if path.exists() and digest and hashlib.new(algorithm,path.read_bytes()).hexdigest()==digest:return path
    temporary=path.with_suffix(path.suffix+'.partial')
    try:
        with urllib.request.urlopen(url,timeout=60) as source,temporary.open('wb') as dest:shutil.copyfileobj(source,dest)
        if digest and hashlib.new(algorithm,temporary.read_bytes()).hexdigest()!=digest:raise ValueError('Download checksum mismatch')
        temporary.replace(path)
    finally:temporary.unlink(missing_ok=True)
    return path

def safe_zip(path,dest):
    dest=dest.resolve()
    with zipfile.ZipFile(path) as archive:
        for member in archive.infolist():
            target=(dest/member.filename).resolve()
            if not target.is_relative_to(dest) or (member.external_attr>>16)&0o170000==0o120000:raise ValueError('Unsafe SDK archive path')
        archive.extractall(dest)

def sdk_at(path=None):
    if path:return Path(path).expanduser().resolve()
    dest=CACHE/'android-sdk'
    if not (dest/'android-35/android.jar').exists() or not (dest/'android-15/lib/d8.jar').exists():
        for name,(url,digest) in SDK_FILES.items():
            print(f'Downloading official Android {name}...',flush=True)
            archive=fetch(url,CACHE/'downloads'/f'{name}.zip',digest,'sha1');safe_zip(archive,dest)
    return dest

def java_at(path=None):
    candidates=[]
    if path:candidates.append(Path(path).expanduser())
    if os.environ.get('JAVA_HOME'):candidates.append(Path(os.environ['JAVA_HOME'])/'bin')
    javac=shutil.which('javac')
    if javac:candidates.append(Path(javac).resolve().parent)
    for home in ['/opt/homebrew/opt/openjdk','/usr/local/opt/openjdk']:
        candidates.append(Path(home)/'bin')
    candidates.extend(CACHE.glob('jdk/**/Contents/Home/bin'))
    for candidate in candidates:
        try:
            subprocess.run([str(candidate/'javac'),'-version'],check=True,stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL,timeout=10)
            return candidate.resolve()
        except (OSError,subprocess.SubprocessError):pass
    if path:raise RuntimeError('Specified JDK does not provide javac')
    arch={'arm64':'aarch64','x86_64':'x64'}.get(platform.machine())
    if not arch:raise RuntimeError('Unsupported Mac CPU; supply --java-bin')
    url=f'https://api.adoptium.net/v3/assets/latest/17/hotspot?architecture={arch}&image_type=jdk&os=mac'
    with urllib.request.urlopen(url,timeout=60) as response:assets=json.load(response)
    package=assets[0]['binary']['package']
    if not package['link'].startswith('https://github.com/adoptium/'):raise ValueError('Unexpected JDK download host')
    print('Downloading local Temurin JDK...',flush=True)
    archive=fetch(package['link'],CACHE/'downloads/jdk.tar.gz',package['checksum'])
    dest=CACHE/'jdk';dest.mkdir(exist_ok=True)
    with tarfile.open(archive) as tar:tar.extractall(dest,filter='data')
    candidates=list(dest.glob('**/Contents/Home/bin'))
    if len(candidates)!=1:raise RuntimeError('Ambiguous JDK archive')
    return candidates[0]

def adb_at(path=None):
    candidate=path or shutil.which('adb')
    if candidate:
        candidate=Path(candidate).expanduser().resolve()
        subprocess.run([str(candidate),'version'],check=True,stdout=subprocess.DEVNULL,timeout=10)
        return candidate
    dest=CACHE/'platform-tools'
    if not (dest/'adb').exists():
        # Google SDK repository supplies archive identity and checksum.
        import xml.etree.ElementTree as ET
        with urllib.request.urlopen('https://dl.google.com/android/repository/repository2-1.xml',timeout=60) as response:tree=ET.fromstring(response.read())
        package=next(e for e in tree.iter() if e.tag.endswith('remotePackage') and e.get('path')=='platform-tools')
        item=next(e for e in package.iter() if e.tag.endswith('archive') and any(x.tag.endswith('host-os') and x.text=='macosx' for x in e))
        url=next(e.text for e in item.iter() if e.tag.endswith('url'))
        checksum=next(e.text for e in item.iter() if e.tag.endswith('checksum'))
        if '/' in url or not url.startswith('platform-tools'):raise ValueError('Unexpected ADB archive')
        archive=fetch('https://dl.google.com/android/repository/'+url,CACHE/'downloads/adb.zip',checksum,'sha1')
        safe_zip(archive,CACHE)
    (dest/'adb').chmod(0o755)
    return dest/'adb'

def venv_at():
    python=ROOT/'.venv/bin/python3'
    if not python.exists():subprocess.run([sys.executable,'-m','venv',str(ROOT/'.venv')],check=True)
    subprocess.run([str(python),'-m','pip','install','--disable-pip-version-check','--only-binary=:all:','--require-hashes','-r',str(ROOT/'requirements-camera.lock')],check=True)
    return python

def build_swift(extend):
    subprocess.run(['xcrun','--find','swiftc'],check=True,stdout=subprocess.DEVNULL)
    out=ROOT/'android-camera/tools';out.mkdir(parents=True,exist_ok=True)
    compiler=subprocess.run(['swiftc','--version'],check=True,capture_output=True,text=True).stdout.strip()
    sources=[('probe',ROOT/'android-camera/probe.swift'),('extract',ROOT/'android-camera/extract.swift'),('read-cells',ROOT/'android-camera/read_cells.swift'),('optical-patches',extend/'benchmarks/display/workloads/optical-patches.swift'),('campaign-workload',extend/'benchmarks/display/workloads/campaign-workload.swift')]
    for name,source in sources:
        target=out/name
        record=out/f'{name}.build.json'
        identity={'source_sha256':hashlib.sha256(source.read_bytes()).hexdigest(),'compiler':compiler,'macos':platform.mac_ver()[0],'architecture':platform.machine()}
        previous=json.loads(record.read_text()) if record.exists() else {}
        valid=target.exists() and all(previous.get(k)==v for k,v in identity.items()) and previous.get('binary_sha256')==hashlib.sha256(target.read_bytes()).hexdigest()
        if not valid:
            print(f'Building {name}...',flush=True)
            with (out/f'{name}-build.log').open('w') as log:
                result=subprocess.run(['swiftc','-O','-module-cache-path',str(out/'module-cache'),str(source),'-o',str(target)],stdout=log,stderr=subprocess.STDOUT)
            if result.returncode:raise RuntimeError(f'Swift build failed; see {out/name}-build.log')
            record.write_text(json.dumps({**identity,'binary_sha256':hashlib.sha256(target.read_bytes()).hexdigest()},indent=2))
    return out
