#!/usr/bin/env bash
# Downloads public .docx test documents into ./corpus (git-ignored).
#
# Each source is a sparse, blob-less, depth-1 clone that checks out only the
# Word files, so even very large repositories cost little. The corpus is not
# committed: the files belong to their projects and carry their licences.
#
#   tools/corpus/fetch.sh            # all sources
#   tools/corpus/fetch.sh poi pandoc # only the named ones
set -euo pipefail

cd "$(dirname "$0")/../.."
mkdir -p corpus

# name  repository
SOURCES=(
  "poi          https://github.com/apache/poi"
  "libreoffice  https://github.com/LibreOffice/core"
  "pandoc       https://github.com/jgm/pandoc"
  "python-docx  https://github.com/python-openxml/python-docx"
  "docx4j       https://github.com/plutext/docx4j"
  "tika         https://github.com/apache/tika"
  "mammoth      https://github.com/mwilliamson/mammoth.js"
  "openxml-sdk  https://github.com/dotnet/Open-XML-SDK"
  "onlyoffice   https://github.com/ONLYOFFICE/document-templates"
)

for entry in "${SOURCES[@]}"; do
  read -r name url <<<"$entry"
  if [ "$#" -gt 0 ] && [[ ! " $* " == *" $name "* ]]; then
    continue
  fi
  dir="corpus/$name"
  if [ ! -d "$dir/.git" ]; then
    git clone --quiet --depth 1 --filter=blob:none --no-checkout "$url" "$dir"
    git -C "$dir" sparse-checkout set --no-cone '*.docx' '*.docm' '*.dotx' '*.dotm'
    git -C "$dir" checkout --quiet
  fi
  count=$(find "$dir" -type f \( -iname '*.docx' -o -iname '*.docm' -o -iname '*.dotx' -o -iname '*.dotm' \) | wc -l)
  printf '%-12s %6d files  %s @ %s\n' "$name" "$count" "$url" "$(git -C "$dir" rev-parse --short=12 HEAD)"
done | tee corpus/SOURCES.txt
