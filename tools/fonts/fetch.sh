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

# name  repository  branch  files...
fetch() {
  local name=$1 url=$2 branch=$3
  shift 3
  local dir="fonts/.src/$name"
  if [ ! -d "$dir/.git" ]; then
    git clone --quiet --depth 1 --branch "$branch" --filter=blob:none --no-checkout "$url" "$dir"
    git -C "$dir" sparse-checkout set --no-cone "$@"
    git -C "$dir" checkout --quiet
  fi
  for file in "$@"; do
    cp "$dir/$file" fonts/
    printf '%-34s %s @ %s\n' "$(basename "$file")" "$url" "$(git -C "$dir" rev-parse --short=12 HEAD)"
  done
}

{
  fetch source-han-serif https://github.com/adobe-fonts/source-han-serif release \
    /SubsetOTF/CN/SourceHanSerifCN-Regular.otf /SubsetOTF/CN/SourceHanSerifCN-Bold.otf
  fetch source-han-sans https://github.com/adobe-fonts/source-han-sans release \
    /SubsetOTF/CN/SourceHanSansCN-Regular.otf /SubsetOTF/CN/SourceHanSansCN-Bold.otf
  fetch lxgw-wenkai https://github.com/lxgw/LxgwWenKai main \
    /fonts/TTF/LXGWWenKai-Regular.ttf
} | tee fonts/SOURCES.txt
