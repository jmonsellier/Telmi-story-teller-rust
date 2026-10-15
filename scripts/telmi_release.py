"""Outils de publication : correctif de runtime.sh et archives de Telmi OS.

    python3 -I scripts/telmi_release.py patch-runtime <runtime.sh d'origine> <fonction> <sortie>
    python3 -I scripts/telmi_release.py repack <zip officiel> <zip produit> <storyTeller> <fonction>

`repack` reprend une archive officielle de Telmi OS à l'identique, sauf :
- `.tmp_update/bin/storyTeller` : binaire Rust (l'original devient `storyTeller.telmi`) ;
- `.tmp_update/runtime.sh` : relance et repli (l'original devient `runtime.sh.telmi`).
"""

import re
import sys
import time
import zipfile

BINARY = ".tmp_update/bin/storyTeller"
RUNTIME = ".tmp_update/runtime.sh"
LAUNCH = re.compile(r"^launch_storyteller\(\) \{\n.*?^\}", re.S | re.M)


def patch_runtime(original: str, function: str) -> str:
    if len(LAUNCH.findall(original)) != 1:
        sys.exit("launch_storyteller introuvable dans runtime.sh : abandon")
    return LAUNCH.sub(lambda _: function.rstrip("\n"), original)


def entry(template: zipfile.ZipInfo, name: str) -> zipfile.ZipInfo:
    info = zipfile.ZipInfo(name, date_time=time.localtime()[:6])
    info.external_attr = template.external_attr
    info.create_system = template.create_system
    info.compress_type = zipfile.ZIP_DEFLATED
    return info


def repack(source: str, target: str, binary_path: str, function_path: str) -> None:
    binary = open(binary_path, "rb").read()
    function = open(function_path, encoding="utf-8").read()
    replaced = set()
    with zipfile.ZipFile(source) as src, zipfile.ZipFile(target, "w", zipfile.ZIP_DEFLATED) as out:
        for info in src.infolist():
            data = src.read(info)
            if info.filename == BINARY:
                out.writestr(entry(info, BINARY + ".telmi"), data)
                out.writestr(entry(info, BINARY), binary)
                replaced.add(BINARY)
            elif info.filename == RUNTIME:
                patched = patch_runtime(data.decode("utf-8"), function)
                out.writestr(entry(info, RUNTIME + ".telmi"), data)
                out.writestr(entry(info, RUNTIME), patched.encode("utf-8"))
                replaced.add(RUNTIME)
            else:
                out.writestr(info, data)
    missing = {BINARY, RUNTIME} - replaced
    if missing:
        sys.exit(f"{source} : fichiers absents {sorted(missing)}")


def main() -> None:
    command, *args = sys.argv[1:] or [""]
    if command == "patch-runtime" and len(args) == 3:
        original, function, output = args
        text = patch_runtime(open(original, newline="").read(), open(function).read())
        open(output, "w", newline="\n").write(text)
    elif command == "repack" and len(args) == 4:
        repack(*args)
    else:
        sys.exit(__doc__)


if __name__ == "__main__":
    main()
