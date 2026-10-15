#!/bin/sh
# Remet le storyTeller et le runtime.sh d'origine de Telmi OS.
set -eu
SD="${1:-/Volumes/ONION}"
UPD="$SD/.tmp_update"
[ -f "$UPD/bin/storyTeller.telmi" ] && [ -f "$UPD/runtime.sh.telmi" ] || { echo "Aucune sauvegarde dans $SD" >&2; exit 1; }
cp -X "$UPD/bin/storyTeller.telmi" "$UPD/bin/storyTeller"
cp -X "$UPD/runtime.sh.telmi" "$UPD/runtime.sh"
sync
echo "Telmi OS d'origine restauré dans $SD."
