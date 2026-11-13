#!/bin/sh
# Builds the browser viewer in web/: the core as oco.wasm, plus links to the
# fonts and the sample documents so that a plain file server can serve it.
#
#   tools/web/build.sh && python3 -m http.server -d web 8000
#
# Fonts are listed in the order the command-line tool loads them, so that a
# page in the browser comes out exactly as `oco render` draws it.
set -e
cd "$(dirname "$0")/../.."

cargo build -p oco-wasm --target wasm32-unknown-unknown --profile wasm
cp target/wasm32-unknown-unknown/wasm/oco_wasm.wasm web/oco.wasm

rm -rf web/fonts web/samples
mkdir -p web/fonts web/samples
list=""
for dir in fonts /usr/share/fonts/truetype/liberation /usr/share/fonts/truetype/crosextra /usr/share/fonts/truetype/dejavu; do
    [ -d "$dir" ] || continue
    for file in $(ls "$dir" | LC_ALL=C sort); do
        case "$file" in
            *.ttf | *.otf | *.ttc | *.TTF | *.OTF | *.TTC) ;;
            *) continue ;;
        esac
        ln -sf "$(cd "$dir" && pwd)/$file" "web/fonts/$file"
        list="$list${list:+,}\"$file\""
    done
done
printf '[%s]\n' "$list" > web/fonts/index.json

list=""
for file in $(ls testdata/zh | LC_ALL=C sort); do
    case "$file" in *.docx) ;; *) continue ;; esac
    ln -sf "$(pwd)/testdata/zh/$file" "web/samples/$file"
    list="$list${list:+,}\"$file\""
done
printf '[%s]\n' "$list" > web/samples/index.json

ls -l web/oco.wasm | awk '{printf "web/oco.wasm: %d KB", $5 / 1024}'
gzip -9 -c web/oco.wasm | wc -c | awk '{printf " (%d KB gzipped)\n", $1 / 1024}'
