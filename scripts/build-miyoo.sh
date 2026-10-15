#!/bin/sh
# Compile storyTeller pour la Miyoo Mini (ARMv7, glibc 2.28) depuis un Mac.
# Prérequis : rustup target add armv7-unknown-linux-gnueabihf, brew install zig,
#             cargo install cargo-zigbuild
set -eu
cd "$(dirname "$0")/.."

# libSDL2.so de Telmi OS (pilotes mmiyoo), utilisée seulement pour l'édition de liens.
TELMI_COMMIT=465fdb41bc83d628266e3e349ca2686ff0d301a6
LIB_DIR="$PWD/target/miyoo-lib"
if [ ! -f "$LIB_DIR/libSDL2.so" ]; then
    mkdir -p "$LIB_DIR"
    curl -fsSL -o "$LIB_DIR/libSDL2.so" \
        "https://raw.githubusercontent.com/DantSu/Telmi-story-teller/$TELMI_COMMIT/lib/libSDL2.so"
fi

MIYOO_LIB_DIR="$LIB_DIR" cargo zigbuild --release -p telmi-os \
    --target armv7-unknown-linux-gnueabihf.2.28
BIN=target/armv7-unknown-linux-gnueabihf/release/storyTeller
mkdir -p dist
cp "$BIN" dist/storyTeller
echo "→ dist/storyTeller ($(du -h dist/storyTeller | cut -f1))"
