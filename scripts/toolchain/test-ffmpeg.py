#!/usr/bin/env python3
"""Real decode/trim/preview checks. Host ffmpeg creates codec fixtures only."""
import json, math, pathlib, struct, subprocess, wave
root = pathlib.Path(__file__).resolve().parents[2]
base = root / '.tools/ffmpeg'
ffmpeg = base / 'install/bin/ffmpeg'
probe = base / 'install/bin/ffprobe'
assert ffmpeg.is_file(), f'Missing scoped binary: {ffmpeg}'
fixtures = base / 'tests'
fixtures.mkdir(parents=True, exist_ok=True)
def run(args):
    p = subprocess.run([str(a) for a in args], capture_output=True, text=True, timeout=60)
    assert p.returncode == 0, f'{args}\n{p.stdout}\n{p.stderr}'
    return p.stdout + p.stderr
source = fixtures / 'source.wav'
with wave.open(str(source), 'wb') as out:
    out.setparams((2, 2, 48000, 0, 'NONE', 'not compressed'))
    out.writeframes(b''.join(struct.pack('<hh', int(12000*math.sin(2*math.pi*440*n/48000)), int(8000*math.sin(2*math.pi*660*n/48000))) for n in range(96000)))
encodings = [('mp3','libmp3lame'),('m4a','aac'),('aac','aac'),('flac','flac'),('ogg','libvorbis'),('opus','libopus'),('webm','libopus'),('mka','flac')]
results = []
for ext, encoder in encodings:
    path = fixtures / f'input.{ext}'
    run(['/usr/bin/ffmpeg','-nostdin','-v','error','-y','-i',source,'-c:a',encoder,path])
for source_path in [source] + [fixtures / f'input.{e}' for e, _ in encodings]:
    info = json.loads(run([probe,'-v','error','-show_streams','-show_format','-of','json',source_path]))
    assert any(s['codec_type']=='audio' for s in info['streams'])
    output = fixtures / f'{source_path.suffix[1:]}-trim.wav'
    run([ffmpeg,'-nostdin','-v','error','-y','-i',source_path,'-vn','-af','asetpts=PTS-STARTPTS,atrim=start_sample=12000:end_sample=60000,asetpts=PTS-STARTPTS','-ar','44100','-ac','1','-c:a','pcm_s24le',output])
    data = output.read_bytes()
    assert data[:4]==b'RIFF' and data[8:12]==b'WAVE'
    chunks = {}; pos = 12
    while pos+8 <= len(data):
        tag, size = struct.unpack_from('<4sI',data,pos)
        chunks[tag] = data[pos+8:pos+8+size]
        pos += 8 + size + (size & 1)
    fmt = chunks[b'fmt ']
    _, channels, rate, _, align, bits = struct.unpack_from('<HHIIHH',fmt)
    assert (channels, rate, align, bits)==(1,44100,3,24)
    assert len(chunks[b'data'])//3 == 44100
    assert any(chunks[b'data']), 'Unexpected silent result'
    preview = fixtures / f'{source_path.suffix[1:]}-preview.wav'
    run([ffmpeg,'-nostdin','-v','error','-y','-i',source_path,'-vn','-ac','1','-ar','44100','-c:a','pcm_s16le',preview])
    with wave.open(str(preview),'rb') as w:
        assert (w.getnchannels(),w.getsampwidth(),w.getframerate())==(1,2,44100)
    results.append({'input':source_path.name,'codec':info['streams'][0]['codec_name'],'output_samples':44100,'preview':'pcm_s16le'})
protocols = run([ffmpeg,'-hide_banner','-protocols'])
assert 'http' not in protocols and 'tcp' not in protocols and 'file' in protocols and 'pipe' in protocols
license_text = run([ffmpeg,'-L'])
assert 'GNU Lesser General Public License' in ' '.join(license_text.split())
assert '--enable-gpl' not in run([ffmpeg,'-buildconf'])
(base/'evidence/functional-tests.json').write_text(json.dumps({'results':results,'protocols':protocols,'status':'passed'},indent=2)+'\n')
print(json.dumps(results,indent=2))
print('PASS: 9 input containers/codecs; exact 44100-sample trim; PCM24 export; PCM16 preview; file/pipe-only; LGPL')
