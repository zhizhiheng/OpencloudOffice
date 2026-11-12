#!/usr/bin/env bash
# Builds the set of documents that have Word's own layout to compare with:
# docx4j's layout probes, each beside the PDF that Word 365 exported of it.
#
# docx4j publishes the PDFs (docx4j-layout-fidelity/goldens/word) but not the
# documents: those are written by its generator, which this script builds and
# runs. Word laid the probes out with Liberation, Carlito and DejaVu installed,
# the same fonts our engine uses, so the two can be compared line for line.
#
#   corpus-word/docx4j/<id>.docx        a probe document
#   corpus-word/docx4j/word/<id>.pdf    Word's PDF of it
#
# Needs git, a JDK and Maven. Downloads about 200 MB of sources plus Maven
# dependencies, and takes about five minutes. Nothing is committed: the
# output is git-ignored, like ./corpus.
#
#   tools/corpus/docx4j_probes.sh
set -euo pipefail

cd "$(dirname "$0")/../.."
# The same commit as the docx4j test documents in ./corpus.
commit=$(awk '$1 == "\"docx4j" { gsub(/"/, "", $3); print $3 }' tools/corpus/fetch.sh)
[ -n "$commit" ] || { echo "no docx4j commit in tools/corpus/fetch.sh" >&2; exit 1; }

out=corpus-word/docx4j
src=corpus-word/.src/docx4j
mkdir -p "$src"
if [ ! -d "$src/.git" ]; then
  git init --quiet "$src"
  git -C "$src" remote add origin https://github.com/plutext/docx4j
fi
if [ "$(git -C "$src" rev-parse --quiet --verify HEAD 2>/dev/null || true)" != "$commit" ]; then
  git -C "$src" fetch --quiet --depth 1 origin "$commit"
  git -C "$src" checkout --quiet --detach FETCH_HEAD
fi

# Maven does not read HTTPS_PROXY; hand it the proxy when there is one.
maven=(mvn --batch-mode --no-transfer-progress -DskipTests -Dgpg.skip=true)
if [[ "${HTTPS_PROXY:-}" =~ ^https?://([^:/]+):([0-9]+) ]]; then
  settings=$(pwd)/corpus-word/.src/maven-settings.xml
  cat >"$settings" <<XML
<settings><proxies><proxy>
  <id>environment</id><active>true</active><protocol>https</protocol>
  <host>${BASH_REMATCH[1]}</host><port>${BASH_REMATCH[2]}</port>
</proxy></proxies></settings>
XML
  maven+=(--settings "$settings")
fi

(
  cd "$src"
  # docx4j-xjc-copy is named because the XJC plugin loads it as a plugin
  # dependency, which --also-make does not see.
  "${maven[@]}" --quiet install --also-make \
    --projects docx4j-xjc-copy,docx4j-export-fo,docx4j-documents4j-local,docx4j-JAXB-ReferenceImpl
  cd docx4j-layout-fidelity
  "${maven[@]}" --quiet package
  rm -rf target/corpus
  java -cp "target/classes:target/lib/*" org.docx4j.fidelity.Fidelity generate target/corpus >/dev/null
)

rm -rf "$out"
mkdir -p "$out/word"
goldens=$src/docx4j-layout-fidelity/goldens/word
for pdf in "$goldens"/*.pdf; do
  id=$(basename "$pdf" .pdf)
  probe=$src/docx4j-layout-fidelity/target/corpus/$id.docx
  if [ -f "$probe" ]; then
    cp "$probe" "$out/"
    cp "$pdf" "$out/word/"
  fi
done
cp "$goldens/golden-manifest.properties" "$out/word/"
printf 'docx4j @ %s: %d probe documents with Word PDFs\n' "$commit" "$(find "$out" -maxdepth 1 -name '*.docx' | wc -l)" |
  tee "$out/SOURCE.txt"
