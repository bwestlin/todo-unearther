#!/usr/bin/env bash
set -euo pipefail

repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
demo_tmp=$(mktemp -d)
cd "$repo_root"
font=DejaVu-Sans-Mono
font_family='DejaVu Sans Mono'
if [[ -f /usr/share/fonts/truetype/firacode/FiraCode-Regular.ttf ]]; then
  font=/usr/share/fonts/truetype/firacode/FiraCode-Regular.ttf
  font_family='Fira Code'
fi
cargo run --locked --quiet --example demo -- "$demo_tmp" "$font_family"
mkdir -p "$repo_root/assets"

frames=()
for frame in "$demo_tmp"/frame-*.svg; do
  name=$(basename "$frame" .svg)
  magick -background none -font "$font" "MSVG:$frame" "$demo_tmp/$name.png"
  frames+=( -delay 340 "$demo_tmp/$name.png" )
done
magick "${frames[@]}" -loop 0 -layers Optimize "$repo_root/assets/todo-unearther-demo.gif"
printf 'Rendered %s\nIntermediate scenes: %s\n' "$repo_root/assets/todo-unearther-demo.gif" "$demo_tmp"
