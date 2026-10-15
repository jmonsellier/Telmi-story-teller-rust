#!/bin/sh
# Installe le storyTeller Rust sur une carte SD Telmi OS (depuis le Mac).
# Usage : scripts/install-sd.sh [/Volumes/ONION]
# Retour arrière : scripts/uninstall-sd.sh
set -eu
HERE="$(cd "$(dirname "$0")" && pwd)"
SD="${1:-/Volumes/ONION}"
UPD="$SD/.tmp_update"
BIN="$HERE/../dist/storyTeller"
MARKER="storyTeller-rs.log"   # chaîne présente uniquement dans notre binaire

[ -f "$UPD/runtime.sh" ] && [ -f "$UPD/bin/storyTeller" ] || { echo "Pas de Telmi OS dans $SD" >&2; exit 1; }
[ -f "$BIN" ] || { echo "Binaire absent : lancer scripts/build-miyoo.sh" >&2; exit 1; }

# Sauvegarde des originaux, rafraîchie si Telmi OS a été mis à jour entre-temps.
if ! grep -q "$MARKER" "$UPD/bin/storyTeller"; then
    cp -X "$UPD/bin/storyTeller" "$UPD/bin/storyTeller.telmi"
fi
if ! grep -q "telmi-rs" "$UPD/runtime.sh"; then
    cp -X "$UPD/runtime.sh" "$UPD/runtime.sh.telmi"
fi

cp -X "$BIN" "$UPD/bin/storyTeller.new"
mv "$UPD/bin/storyTeller.new" "$UPD/bin/storyTeller"

# Remplace la fonction launch_storyteller de runtime.sh (toujours depuis l'original).
python3 -I - "$UPD/runtime.sh.telmi" "$HERE/launch_storyteller.sh" "$UPD/runtime.sh.new" <<'PY'
import re, sys
original, function, target = sys.argv[1:4]
text = open(original, newline="").read()
new = open(function).read().rstrip("\n")
pattern = re.compile(r"^launch_storyteller\(\) \{\n.*?^\}", re.S | re.M)
if len(pattern.findall(text)) != 1:
    sys.exit("launch_storyteller introuvable dans runtime.sh : installation annulée")
open(target, "w", newline="\n").write(pattern.sub(lambda _: new, text))
PY
mv "$UPD/runtime.sh.new" "$UPD/runtime.sh"
rm -f "$UPD/bin/._storyTeller" "$UPD/._runtime.sh"
sync
echo "Installé dans $SD (originaux : bin/storyTeller.telmi, runtime.sh.telmi)."
echo "Journal sur la console : .tmp_update/logs/storyTeller-rs.log"
