#!/usr/bin/env bash
# Turns the frames recorded by ci/demo.ps1 into README GIFs and copies the
# stills and the speed comparison next to them.
#   usage: ci/make-media.sh <media-raw dir> <output dir>
set -euo pipefail
raw=$1 out=$2
mkdir -p "$out"
for dir in "$raw"/frames-*; do
  [ -d "$dir" ] || continue
  lang=${dir##*/frames-}
  ffmpeg -loglevel error -y -framerate 8 -i "$dir/%04d.png" \
    -vf "scale=800:-1:flags=lanczos,split[a][b];[a]palettegen=max_colors=128:stats_mode=diff[p];[b][p]paletteuse=dither=bayer:bayer_scale=4:diff_mode=rectangle" \
    -loop 0 "$out/demo-$lang.gif"
  echo "demo-$lang.gif: $(du -h "$out/demo-$lang.gif" | cut -f1)"
done
for f in "$raw"/*.png "$raw"/bench.md "$raw"/bench.json; do
  [ -f "$f" ] && cp "$f" "$out/"
done
ls -la "$out"
