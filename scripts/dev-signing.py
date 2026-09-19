#!/usr/bin/env python3
"""Local development certificate management. Never used by release builds."""
import contextlib
import hashlib
import os
from pathlib import Path
import secrets
import shlex
import shutil
import subprocess
import tempfile

ROOT = Path.home() / 'Library/Application Support/extend.computer Development Signing'
KEYCHAIN = ROOT / 'extend-computer-development.keychain-db'
CERT = ROOT / 'certificate.pem'
PASSWORD = ROOT / 'keychain-password'


def run(*args, input=None):
    result = subprocess.run([str(a) for a in args], input=input, text=True, capture_output=True)
    if result.returncode:
        # Do not put command arguments (which may contain keychain secrets) in exceptions.
        raise RuntimeError(f'{Path(str(args[0])).name} failed: {result.stderr.strip()}')
    return result.stdout


def password():
    if PASSWORD.is_symlink() or PASSWORD.stat().st_mode & 0o077:
        raise RuntimeError('Unsafe development keychain password permissions')
    return PASSWORD.read_text()


def fingerprint():
    return hashlib.sha1(__import__('ssl').PEM_cert_to_DER_cert(CERT.read_text())).hexdigest().upper()


@contextlib.contextmanager
def signing_keychain():
    import fcntl
    with (ROOT / 'signing.lock').open('a') as lock:
        fcntl.flock(lock, fcntl.LOCK_EX)
        run('/usr/bin/security', 'unlock-keychain', '-p', password(), KEYCHAIN)
        previous = shlex.split(run('/usr/bin/security', 'list-keychains', '-d', 'user'))
        try:
            run('/usr/bin/security', 'list-keychains', '-d', 'user', '-s', *dict.fromkeys([*previous, str(KEYCHAIN)]))
            yield
        finally:
            run('/usr/bin/security', 'list-keychains', '-d', 'user', '-s', *previous)


def setup():
    if ROOT.is_symlink():
        raise RuntimeError('Development signing directory must not be a symlink')
    ROOT.mkdir(parents=True, mode=0o700, exist_ok=True)
    ROOT.chmod(0o700)
    if not PASSWORD.exists():
        with PASSWORD.open('x') as file:
            os.chmod(PASSWORD, 0o600)
            file.write(secrets.token_urlsafe(40))
    secret = password()
    if not CERT.exists():
        if KEYCHAIN.exists():
            raise RuntimeError('Existing signing keychain has no certificate; refusing to replace its identity')
        openssl = shutil.which('openssl')
        if not openssl:
            raise RuntimeError('OpenSSL is required for one-time development signing setup')
        with tempfile.TemporaryDirectory(dir=ROOT) as temporary:
            temp = Path(temporary)
            config = temp / 'certificate.cnf'
            config.write_text('[req]\ndistinguished_name=dn\nx509_extensions=extensions\nprompt=no\n[dn]\nCN=extend.computer Local Development\n[extensions]\nbasicConstraints=critical,CA:FALSE\nkeyUsage=critical,digitalSignature\nextendedKeyUsage=codeSigning\n')
            run(openssl, 'req', '-x509', '-newkey', 'rsa:3072', '-nodes', '-days', '3650', '-config', config, '-keyout', temp/'private.pem', '-out', CERT)
            previous = shlex.split(run('/usr/bin/security', 'list-keychains', '-d', 'user'))
            try:
                run('/usr/bin/security', 'create-keychain', '-p', secret, KEYCHAIN)
            finally:
                run('/usr/bin/security', 'list-keychains', '-d', 'user', '-s', *previous)
            run(openssl, 'pkcs12', '-export', '-inkey', temp/'private.pem', '-in', CERT, '-out', temp/'identity.p12', '-passout', 'stdin', '-keypbe', 'PBE-SHA1-3DES', '-certpbe', 'PBE-SHA1-3DES', '-macalg', 'sha1', input=secret+'\n')
            run('/usr/bin/security', 'import', temp/'identity.p12', '-k', KEYCHAIN, '-P', secret, '-T', '/usr/bin/codesign', '-x')
            run('/usr/bin/security', 'set-key-partition-list', '-S', 'apple-tool:,apple:', '-s', '-k', secret, KEYCHAIN)
    if not KEYCHAIN.exists():
        raise RuntimeError('Development signing keychain missing; refusing to generate a different identity')
    run('/usr/bin/security', 'unlock-keychain', '-p', secret, KEYCHAIN)
    identities = run('/usr/bin/security', 'find-identity', '-v', '-p', 'codesigning', KEYCHAIN)
    if fingerprint() not in identities:
        # User-domain code-signing trust only. No TLS/system trust changes.
        run('/usr/bin/security', 'add-trusted-cert', '-r', 'trustRoot', '-p', 'codeSign', '-k', KEYCHAIN, CERT)
    print('Development signing identity ready:', fingerprint())


def sign(path, identifier):
    if not identifier.endswith('.development'):
        raise RuntimeError('Refusing to dev-sign a production bundle identifier')
    pin = fingerprint()
    requirement = f'identifier "{identifier}" and certificate leaf = H"{pin}"'
    run('/usr/bin/codesign', '--force', '--sign', pin, '--keychain', KEYCHAIN, '--timestamp=none', '--identifier', identifier, '--requirements', '=designated => ' + requirement, path)
    run('/usr/bin/codesign', '--verify', '--strict', '-R', '=' + requirement, path)
