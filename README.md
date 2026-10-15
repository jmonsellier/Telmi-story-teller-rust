# MiyooKids — storyTeller en Rust

Remplaçant robuste du binaire `storyTeller` de [Telmi OS](https://github.com/DantSu/Telmi-story-teller)
pour Miyoo Mini Plus. Il garde tout le reste de Telmi OS (démarrage, batterie, Telmi Sync, format
de la carte SD, sauvegardes) et ne remplace que la conteuse.

## Pourquoi

Avec une grosse bibliothèque (30 Go, collections de 10 Mo de `nodes.json`), Telmi OS plantait
dans les menus. Causes trouvées dans son code :

| Problème dans Telmi OS | Ici |
|---|---|
| `nodes.json` chargé en arbre cJSON : ≈ 46 Mo de RAM pour 10,9 Mo de JSON | lecture en flux vers une forme compacte : 4 Mo gardés, pic de 12 Mo |
| `cJSON_free` au lieu de `cJSON_Delete` : tout l'arbre fuit à chaque sortie d'histoire | mémoire libérée automatiquement ; 10 allers-retours = +2 Mo |
| cache de 32 images sans limite de taille (jusqu'à 39 Mo) | cache plafonné à 12 Mo |
| verrou audio gardé pendant le parcours d'un mp3 sur la SD (gel) | décodage dans un thread, sortie son sans entrée-sortie |
| boucle principale à 100 % du processeur | attente bloquante des boutons |
| plantage = écran figé jusqu'à épuisement de la batterie | reprise sur panique, relance par `runtime.sh`, repli sur l'original |
| divisions par zéro, index hors limites, `strcpy` sans borne sur des données de la carte | données vérifiées, histoire illisible = retour au titre |

## Organisation

- `crates/story` : moteur d'histoires sans dépendance matérielle (lecture de `nodes.json`,
  navigation, inventaire, sauvegardes compatibles Telmi).
- `crates/os` : binaire `storyTeller` (SDL2 de Telmi, audio, images, texte, boutons, matériel, applications).
- `scripts/` : compilation croisée et installation sur la carte SD.

## Développer

```sh
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings

# Tests sur une vraie bibliothèque Telmi Sync
TELMI_STORIES=~/.telmi/stories cargo test --release -- --nocapture
```

### Simulateur Mac

Nécessite `brew install sdl2`. `TELMI_ROOT` pointe vers une copie de l'arborescence de la carte
(`.tmp_update/res` avec les images et polices de Telmi, `Stories`, `Music`, `Saves`).

```sh
TELMI_ROOT=~/sd cargo run --release -p telmi-os
```

Clavier : flèches, Entrée = A, Retour arrière = Home, Espace = pause, Échap = Menu, `+`/`-` = volume.

Mode scripté (captures PNG de chaque écran) :

```sh
SDL_AUDIODRIVER=dummy TELMI_ROOT=~/sd TELMI_SNAPSHOTS=/tmp/snaps \
TELMI_SCRIPT="wait:1000,A,wait:1500,Right,Menu,A,wait:3000,Quit" ./target/release/storyTeller
```

## Installer sur la console

Prérequis : `rustup target add armv7-unknown-linux-gnueabihf`, `brew install zig`,
`cargo install cargo-zigbuild`.

```sh
scripts/build-miyoo.sh                 # → dist/storyTeller (ARMv7, glibc 2.28)
scripts/install-sd.sh /Volumes/ONION   # sauvegarde les originaux puis installe
scripts/uninstall-sd.sh /Volumes/ONION # retour au Telmi OS d'origine
```

L'installation remplace `.tmp_update/bin/storyTeller` et la fonction `launch_storyteller` de
`runtime.sh` : relance automatique en cas de plantage, puis repli sur `storyTeller.telmi` après
trois échecs. Une mise à jour de Telmi OS par Telmi Sync écrase ces fichiers : relancer
`install-sd.sh`.

Journal sur la console : `.tmp_update/logs/storyTeller-rs.log`.

## Limites connues

- Prise casque de la Miyoo Mini Flip (GPIO 44/45) non gérée : la Mini Plus n'est pas concernée.
- En mode « répéter le titre », haut et bas changent de piste (Telmi rejouait la même).
