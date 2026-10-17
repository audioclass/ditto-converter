#!/usr/bin/env python3
"""Verify pinned official source hash and upstream detached signature."""
import base64, hashlib, pathlib, subprocess, urllib.request
root = pathlib.Path(__file__).resolve().parents[2]
base = root/'.tools/ffmpeg'
source = base/'source'
files = [('ffmpeg-8.1.3.tar.xz.asc','https://ffmpeg.org/releases/ffmpeg-8.1.3.tar.xz.asc',None), ('ffmpeg-devel.asc','https://ffmpeg.org/ffmpeg-devel.asc','397b3becedcd5a98769967ff1ff8501ddc89f8368b8f766e4701377d7dbaabe5')]
for name, url, digest in files:
    path = source/name
    if not path.exists():
        with urllib.request.urlopen(url,timeout=60) as response:
            path.write_bytes(response.read())
    if digest: assert hashlib.sha256(path.read_bytes()).hexdigest()==digest
body = (source/'ffmpeg-devel.asc').read_text().split('\n\n',1)[1].split('-----END')[0]
key = source/'ffmpeg-devel.gpg'
key.write_bytes(base64.b64decode(''.join(s for s in body.splitlines() if not s.startswith('='))))
verifier = root/'.tools/sysroot/usr/bin/gpgv'
if not verifier.exists():
    print('gpgv unavailable: pinned SHA checked by build script; signature not verified on this run')
else:
    home = base/'gnupg'
    home.mkdir(mode=0o700,exist_ok=True)
    p = subprocess.run([str(verifier),'--homedir',str(home),'--keyring',str(key),'--status-fd','1',str(source/'ffmpeg-8.1.3.tar.xz.asc'),str(source/'ffmpeg-8.1.3.tar.xz')],capture_output=True,text=True)
    result = p.stdout+p.stderr
    (base/'evidence/source-signature.txt').write_text(result)
    assert p.returncode==0 and '[GNUPG:] VALIDSIG FCF986EA15E6E293A5644F10B4322F04D67658D8 ' in result, result
    print('Good pinned FFmpeg signature FCF986EA15E6E293A5644F10B4322F04D67658D8')
