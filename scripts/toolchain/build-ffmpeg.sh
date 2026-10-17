#!/usr/bin/env bash
set -euo pipefail
ROOT=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
BASE="$ROOT/.tools/ffmpeg"
VERSION=8.1.3
SHA256=7138d28c96d9d3e3af4ee3d8cad72741f8ffb40da90c1112235dea3ecd3178a3
export TMPDIR="$ROOT/.work/tmp"
export LC_ALL=C SOURCE_DATE_EPOCH=1792195200
mkdir -p "$TMPDIR" "$BASE"/{source,build,install,evidence}
ARCHIVE="$BASE/source/ffmpeg-$VERSION.tar.xz"
if [[ ! -f "$ARCHIVE" ]]; then
  curl --fail --location --retry 3 "https://ffmpeg.org/releases/ffmpeg-$VERSION.tar.xz" -o "$ARCHIVE"
fi
printf '%s  %s\n' "$SHA256" "$ARCHIVE" | sha256sum --check
python3 "$ROOT/scripts/toolchain/verify-ffmpeg-source.py"
[[ -d "$BASE/source/ffmpeg-$VERSION" ]] || tar -xf "$ARCHIVE" -C "$BASE/source"
cd "$BASE/build"
# No accidental linkage to sysroot or host multimedia dependencies.
unset PKG_CONFIG_PATH PKG_CONFIG_SYSROOT_DIR LIBRARY_PATH LD_LIBRARY_PATH
args=(
  --prefix="$BASE/install" --disable-everything --disable-autodetect
  --disable-network --disable-doc --disable-debug --disable-x86asm
  --disable-avdevice --disable-swscale --disable-ffplay
  --disable-shared --enable-static --enable-small --enable-pthreads
  --enable-ffmpeg --enable-ffprobe --enable-avcodec --enable-avformat
  --enable-avfilter --enable-swresample
  --enable-protocol=file,pipe
  '--enable-decoder=mp3,mp3float,mp3adu,mp3adufloat,mp3on4,mp3on4float,aac,aac_fixed,flac,vorbis,opus,alac,pcm*'
  '--enable-demuxer=mp3,aac,mov,flac,wav,aiff,ogg,matroska,pcm*'
  --enable-parser=aac,aac_latm,mpegaudio,flac,vorbis,opus
  --enable-encoder=pcm_s24le,pcm_s16le
  --enable-muxer=wav,pcm_s24le
  --enable-filter=abuffer,abuffersink,aformat,anull,aresample,atrim,asetpts
  --extra-cflags="-ffile-prefix-map=$ROOT=. -fdebug-prefix-map=$ROOT=."
)
printf '%q ' "$BASE/source/ffmpeg-$VERSION/configure" "${args[@]}" > "$BASE/evidence/configure-command.txt"
printf '\n' >> "$BASE/evidence/configure-command.txt"
"$BASE/source/ffmpeg-$VERSION/configure" "${args[@]}" > "$BASE/evidence/configure.log" 2>&1
make -j "${DITTO_BUILD_JOBS:-2}" > "$BASE/evidence/make.log" 2>&1
make install > "$BASE/evidence/install.log" 2>&1
for tool in ffmpeg ffprobe; do
  "$BASE/install/bin/$tool" -version > "$BASE/evidence/$tool-version.txt" 2>&1
  "$BASE/install/bin/$tool" -buildconf > "$BASE/evidence/$tool-buildconf.txt" 2>&1
  "$BASE/install/bin/$tool" -L > "$BASE/evidence/$tool-license.txt" 2>&1
  ldd "$BASE/install/bin/$tool" > "$BASE/evidence/$tool-ldd.txt"
  readelf -d "$BASE/install/bin/$tool" > "$BASE/evidence/$tool-dynamic.txt"
  readelf --version-info "$BASE/install/bin/$tool" > "$BASE/evidence/$tool-symbol-versions.txt"
done
for feature in decoders demuxers encoders muxers filters protocols; do
  "$BASE/install/bin/ffmpeg" -hide_banner -"$feature" > "$BASE/evidence/$feature.txt" 2>&1
done
mkdir -p "$BASE/install/notices"
cp "$BASE/source/ffmpeg-$VERSION"/COPYING* "$BASE/source/ffmpeg-$VERSION/LICENSE.md" "$BASE/install/notices/"
cp "$BASE/build/config.h" "$BASE/build/ffbuild/config.mak" "$BASE/evidence/"
python3 "$ROOT/scripts/toolchain/ffmpeg-inventory.py"
python3 "$ROOT/scripts/toolchain/test-ffmpeg.py" > "$BASE/evidence/test-green.log" 2>&1
printf '%s\n' "Built and tested $BASE/install/bin/{ffmpeg,ffprobe}"
