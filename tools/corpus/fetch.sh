#!/usr/bin/env bash
# Downloads public .docx test documents into ./corpus (git-ignored).
#
# Each source is a sparse, blob-less, depth-1 fetch that checks out only the
# Word files, so even very large repositories cost little. The corpus is not
# committed: the files belong to their projects and carry their licences.
#
# Every source is pinned to a commit. The fidelity scores are counts over
# these files, so they only mean something from one run to the next if the
# files stay the same.
#
#   tools/corpus/fetch.sh            # all sources, at the pinned commits
#   tools/corpus/fetch.sh poi pandoc # only the named ones
#   tools/corpus/fetch.sh --latest   # at the head of each default branch;
#                                    # prints the commits to pin below
set -euo pipefail

cd "$(dirname "$0")/../.."
mkdir -p corpus

# name  repository  commit
SOURCES=(
  "poi          https://github.com/apache/poi                     a4e8640da7c3bb906efc6fa11914d1b80f3cc9dc"
  "libreoffice  https://github.com/LibreOffice/core               b9166e293d413e45ac213dcc1afb6d7cfebd8739"
  "pandoc       https://github.com/jgm/pandoc                     3dde36a9ddd68b8677397d9c59c891d9b8763b89"
  "python-docx  https://github.com/python-openxml/python-docx     e45454602b53e8e572b179ccf1c91093ec9f4ed7"
  "docx4j       https://github.com/plutext/docx4j                 c0a66b6100bb49d93dd0de005f7cdfd3784897ed"
  "tika         https://github.com/apache/tika                    a40d54f6a6d9cf8ce485f10bb07011cd32ef5925"
  "mammoth      https://github.com/mwilliamson/mammoth.js         5171a9072f8b7343ab1c9ed3a15d6d372cb59b67"
  "openxml-sdk  https://github.com/dotnet/Open-XML-SDK            431ab05cf160248cc3885a4a766026d4f8243792"
  "onlyoffice   https://github.com/ONLYOFFICE/document-templates  0a52fb76fa53fddf7d70da28878537cde3d29421"
)

latest=false
names=()
for argument in "$@"; do
  if [ "$argument" = "--latest" ]; then latest=true; else names+=("$argument"); fi
done

for entry in "${SOURCES[@]}"; do
  read -r name url commit <<<"$entry"
  if [ "${#names[@]}" -gt 0 ] && [[ ! " ${names[*]} " == *" $name "* ]]; then
    continue
  fi
  dir="corpus/$name"
  if [ ! -d "$dir/.git" ]; then
    git init --quiet "$dir"
    git -C "$dir" remote add origin "$url"
    git -C "$dir" config remote.origin.promisor true
    git -C "$dir" config remote.origin.partialclonefilter blob:none
    git -C "$dir" sparse-checkout set --no-cone '*.docx' '*.docm' '*.dotx' '*.dotm'
  fi
  if $latest; then
    git -C "$dir" fetch --quiet --depth 1 --filter=blob:none origin HEAD
    git -C "$dir" checkout --quiet --detach FETCH_HEAD
  elif [ "$(git -C "$dir" rev-parse --quiet --verify HEAD 2>/dev/null || true)" != "$commit" ]; then
    git -C "$dir" fetch --quiet --depth 1 --filter=blob:none origin "$commit"
    git -C "$dir" checkout --quiet --detach FETCH_HEAD
  fi
  count=$(find "$dir" -type f \( -iname '*.docx' -o -iname '*.docm' -o -iname '*.dotx' -o -iname '*.dotm' \) | wc -l)
  printf '%-12s %6d files  %s @ %s\n' "$name" "$count" "$url" "$(git -C "$dir" rev-parse HEAD)"
done | tee corpus/SOURCES.txt
