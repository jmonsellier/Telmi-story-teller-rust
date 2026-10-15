# Telmi-story-teller-rust

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

## Installation

Testé sur **Miyoo Mini Plus**, à partir de Telmi OS 1.10.4. Miyoo Mini et Miyoo Mini Flip : non
testés.

### Telmi OS est déjà sur la carte (cas le plus courant)

Les histoires, la musique, les réglages et les reprises en cours sont conservés.

1. Télécharge `TelmiOS_v1.10.4-rust_v…-update.zip` dans la
   [dernière version](https://github.com/jmonsellier/Telmi-story-teller-rust/releases/latest).
2. Décompresse l'archive sur l'ordinateur.
3. Copie le dossier `.tmp_update` qu'elle contient à la racine de la carte SD, en acceptant de
   remplacer l'existant.
   - **Mac** : ce dossier est caché. Dans le Finder, `Cmd` + `Maj` + `.` l'affiche.
   - **Windows** : copier, coller à la racine de la carte, puis « Remplacer les fichiers ».
4. Éjecte la carte, remets-la dans la console et allume-la.

### Nouvelle carte SD

Installe d'abord Telmi OS avec [Telmi Sync](https://telmi.fr/#download), puis suis les étapes
ci-dessus. L'archive `TelmiOS_v1.10.4-rust_v….zip` (sans `-update`) est l'équivalent de l'archive
complète de Telmi OS, pour une installation manuelle sur une carte vide.

### Bon à savoir

- Si la nouvelle conteuse plante trois fois de suite, la console relance d'elle-même celle de
  Telmi OS, gardée sur la carte (`.tmp_update/bin/storyTeller.telmi`).
- Une mise à jour de Telmi OS proposée par Telmi Sync remplace cette version : réapplique ensuite
  l'archive `-update`.
- Pour revenir à Telmi OS d'origine, applique de la même façon l'archive `-update` officielle de
  [Telmi OS](https://github.com/DantSu/Telmi-story-teller/releases).
- En cas de souci, le journal est sur la carte : `.tmp_update/logs/storyTeller-rs.log`.

## Organisation

- `crates/story` : moteur d'histoires sans dépendance matérielle (lecture de `nodes.json`,
  navigation, inventaire, sauvegardes compatibles Telmi).
- `crates/os` : binaire `storyTeller` (SDL2 de Telmi, audio, images, texte, boutons, matériel, applications).
- `scripts/` : compilation croisée, installation sur la carte SD et archives de publication.

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

## Installer depuis les sources

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

### Publier une version

```sh
scripts/make-release.sh 1.10.4   # → dist/release/*.zip et SHA256SUMS
```

Le script reprend les archives officielles de Telmi OS de la version indiquée et n'y remplace
que `storyTeller` et `runtime.sh` (originaux gardés en `.telmi`). Le numéro de version vient de
`crates/os/Cargo.toml`.

## Licence et origine

GPL-3.0, comme Telmi OS dont ce projet reprend la logique (`stories_reader.h`,
`music_player.h`, `app_selector.h`, `storyTeller.c`) et les ressources graphiques.
Telmi OS : © Franck Alary (DantSu) et contributeurs, <https://github.com/DantSu/Telmi-story-teller>.

## Limites connues

- Prise casque de la Miyoo Mini Flip (GPIO 44/45) non gérée : la Mini Plus n'est pas concernée.
- En mode « répéter le titre », haut et bas changent de piste (Telmi rejouait la même).
