#!/usr/bin/env bash
# Downloads the open-licence fonts the layout engine uses in place of the
# proprietary fonts that Chinese documents name (SimSun, SimHei, KaiTi, ...).
# All are under the SIL Open Font License. They go to ./fonts, which is
# git-ignored: the files are large and belong to their projects.
#
#   tools/fonts/fetch.sh
set -euo pipefail

cd "$(dirname "$0")/../.."
mkdir -p fonts/.src

# name  repository  commit  files...
#
# Pinned to a commit: line breaks follow glyph widths, so the layout scores
# are only comparable between runs that used the same font files.
fetch() {
  local name=$1 url=$2 commit=$3
  shift 3
  local dir="fonts/.src/$name"
  if [ ! -d "$dir/.git" ]; then
    git init --quiet "$dir"
    git -C "$dir" remote add origin "$url"
    git -C "$dir" config remote.origin.promisor true
    git -C "$dir" config remote.origin.partialclonefilter blob:none
    git -C "$dir" sparse-checkout set --no-cone "$@"
  fi
  if [ "$(git -C "$dir" rev-parse --quiet --verify HEAD 2>/dev/null || true)" != "$commit" ]; then
    git -C "$dir" fetch --quiet --depth 1 --filter=blob:none origin "$commit"
    git -C "$dir" checkout --quiet --detach FETCH_HEAD
  fi
  for file in "$@"; do
    cp "$dir/$file" fonts/
    printf '%-34s %s @ %s\n' "$(basename "$file")" "$url" "$(git -C "$dir" rev-parse --short=12 HEAD)"
  done
}

{
  fetch source-han-serif https://github.com/adobe-fonts/source-han-serif 7889f11bf31170b5d092a083b357c8c8130f89e0 \
    /SubsetOTF/CN/SourceHanSerifCN-Regular.otf /SubsetOTF/CN/SourceHanSerifCN-Bold.otf
  fetch source-han-sans https://github.com/adobe-fonts/source-han-sans a4f7cf94edfb9d7ffbdfc4841de276358bd7e0f2 \
    /SubsetOTF/CN/SourceHanSansCN-Regular.otf /SubsetOTF/CN/SourceHanSansCN-Bold.otf
  fetch lxgw-wenkai https://github.com/lxgw/LxgwWenKai 8afc526476c2811e2e90e765444e7a6e4a7ecf77 \
    /fonts/TTF/LXGWWenKai-Regular.ttf
} | tee fonts/SOURCES.txt
