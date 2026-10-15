#!/bin/sh
# Construit les archives publiques à partir de celles de Telmi OS.
# Usage : scripts/make-release.sh 1.10.4
# Produit dist/release/TelmiOS_v<telmi>-rust_v<version>[-update].zip et SHA256SUMS.
set -eu
cd "$(dirname "$0")/.."
TELMI="${1:?usage : scripts/make-release.sh <version de Telmi OS, ex. 1.10.4>}"
VERSION=$(sed -n 's/^version = "\(.*\)"/\1/p' crates/os/Cargo.toml | head -1)

./scripts/build-miyoo.sh
OUT=dist/release
rm -rf "$OUT"
mkdir -p "$OUT/official"
for kind in "" "-update"; do
    name="TelmiOS_v${TELMI}${kind}.zip"
    curl -fsSL -o "$OUT/official/$name" \
        "https://github.com/DantSu/Telmi-story-teller/releases/download/$TELMI/$name"
    python3 -I scripts/telmi_release.py repack "$OUT/official/$name" \
        "$OUT/TelmiOS_v${TELMI}-rust_v${VERSION}${kind}.zip" dist/storyTeller scripts/launch_storyteller.sh
done
rm -rf "$OUT/official"
(cd "$OUT" && shasum -a 256 ./*.zip | sed 's| \./| |' > SHA256SUMS)
ls -l "$OUT"
