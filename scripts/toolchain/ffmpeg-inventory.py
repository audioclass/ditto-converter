#!/usr/bin/env python3
import datetime, hashlib, json, pathlib, platform, subprocess
root = pathlib.Path(__file__).resolve().parents[2]
base = root / '.tools/ffmpeg'
files = []
for path in sorted((base/'install').rglob('*')):
    if path.is_file():
        files.append({'path':str(path.relative_to(root)), 'bytes':path.stat().st_size, 'sha256':hashlib.sha256(path.read_bytes()).hexdigest()})
source = base/'source/ffmpeg-8.1.3.tar.xz'
data = {'version':'8.1.3','source_url':'https://ffmpeg.org/releases/ffmpeg-8.1.3.tar.xz','source_sha256':hashlib.sha256(source.read_bytes()).hexdigest(),'built_utc':datetime.datetime.now(datetime.timezone.utc).isoformat(),'platform':platform.platform(),'compiler':subprocess.check_output(['cc','--version'],text=True),'make':subprocess.check_output(['make','--version'],text=True),'source_signature':(base/'evidence/source-signature.txt').read_text() if (base/'evidence/source-signature.txt').exists() else 'Not verified: gpgv unavailable; source hash still pinned', 'source_signing_key_url':'https://ffmpeg.org/ffmpeg-devel.asc', 'source_signing_key_sha256':'397b3becedcd5a98769967ff1ff8501ddc89f8368b8f766e4701377d7dbaabe5','linkage':'FFmpeg internal libraries static; libm and libc dynamic on this host, see ldd evidence','files':files}
(base/'installation-manifest.json').write_text(json.dumps(data,indent=2)+'\n')
print(json.dumps([r for r in files if '/bin/' in r['path']],indent=2))
