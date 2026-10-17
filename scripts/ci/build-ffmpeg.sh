#!/usr/bin/env bash
# Native scoped offline engine; never use this pair as a corpus generator.
set -euo pipefail
ROOT=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
BASE="$ROOT/.work/ci/ffmpeg"
VERSION=8.1.3
SHA256=7138d28c96d9d3e3af4ee3d8cad72741f8ffb40da90c1112235dea3ecd3178a3
export LC_ALL=C SOURCE_DATE_EPOCH=1792195200
export TMPDIR="$ROOT/.work/ci/tmp"
mkdir -p "$BASE"/{source,build,install,evidence} "$TMPDIR"
case "$(uname -s):$(uname -m)" in
  Darwin:arm64) platform=darwin; extra=(--arch=aarch64 --cc=clang --enable-pthreads);;
  MINGW*:*|MSYS*:*)
    export PATH="/ucrt64/bin:/usr/bin:$PATH"
    [[ "$(gcc -dumpmachine)" == x86_64-w64-mingw32 ]] || { echo 'Wrong MinGW target'; exit 1; }
    platform=win32
    extra=(--arch=x86_64 --target-os=mingw32 --cc=gcc --disable-pthreads --enable-w32threads --extra-ldflags=-static-libgcc);;
  *) echo 'Only native Apple Silicon macOS / Windows x64 builds are authorized'; exit 1;;
esac
archive="$BASE/source/ffmpeg-$VERSION.tar.xz"
curl --fail --location --retry 3 "https://ffmpeg.org/releases/ffmpeg-$VERSION.tar.xz" -o "$archive"
# Node runs natively on both platforms; do not depend on sha256sum/shasum parity.
# Convert the archive path for native Windows Node when launched by MSYS2.
archive_native="$archive"
if [[ "$platform" == win32 ]]; then archive_native=$(cygpath -m "$archive"); fi
node -e 'const f=require("node:fs"),c=require("node:crypto");if(c.createHash("sha256").update(f.readFileSync(process.argv[1])).digest("hex")!==process.argv[2])throw Error("FFmpeg source SHA256 mismatch")' "$archive_native" "$SHA256"
tar -xf "$archive" -C "$BASE/source"
unset PKG_CONFIG_PATH PKG_CONFIG_SYSROOT_DIR LIBRARY_PATH LD_LIBRARY_PATH
args=(
  --prefix="$BASE/install" --disable-everything --disable-autodetect
  --disable-network --disable-doc --disable-debug --disable-x86asm
  --disable-avdevice --disable-swscale --disable-ffplay
  --disable-shared --enable-static --enable-small
  --enable-ffmpeg --enable-ffprobe --enable-avcodec --enable-avformat
  --enable-avfilter --enable-swresample --enable-protocol=file,pipe
  '--enable-decoder=mp3,mp3float,mp3adu,mp3adufloat,mp3on4,mp3on4float,aac,aac_fixed,flac,vorbis,opus,alac,pcm*'
  '--enable-demuxer=mp3,aac,mov,flac,wav,aiff,ogg,matroska,pcm*'
  --enable-parser=aac,aac_latm,mpegaudio,flac,vorbis,opus
  --enable-encoder=pcm_s24le,pcm_s16le --enable-muxer=wav,pcm_s24le
  --enable-filter=abuffer,abuffersink,aformat,anull,aresample,atrim,asetpts
  "--extra-cflags=-ffile-prefix-map=$ROOT=. -fdebug-prefix-map=$ROOT=."
  "${extra[@]}"
)
printf '%q ' "$BASE/source/ffmpeg-$VERSION/configure" "${args[@]}" > "$BASE/evidence/configure-command.txt"
printf '\n' >> "$BASE/evidence/configure-command.txt"
cd "$BASE/build"
"$BASE/source/ffmpeg-$VERSION/configure" "${args[@]}" 2>&1 | tee "$BASE/evidence/configure.log"
make -j "${DITTO_BUILD_JOBS:-3}" 2>&1 | tee "$BASE/evidence/make.log"
make install 2>&1 | tee "$BASE/evidence/install.log"
cp config.h ffbuild/config.mak "$BASE/evidence/"
mkdir -p "$BASE/install/notices"
cp "$BASE/source/ffmpeg-$VERSION"/COPYING* "$BASE/source/ffmpeg-$VERSION/LICENSE.md" "$BASE/install/notices/"
if [[ "$platform" == win32 ]]; then
  gcc --version > "$BASE/evidence/compiler.txt"
  pacman -Q > "$BASE/evidence/msys2-packages.txt"
else
  clang --version > "$BASE/evidence/compiler.txt"
  xcodebuild -version > "$BASE/evidence/xcode.txt"
  xcrun --show-sdk-version > "$BASE/evidence/sdk.txt"
fi
make --version > "$BASE/evidence/make-version.txt"
