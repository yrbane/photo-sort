# photo-sort

Outil CLI en Rust qui trie automatiquement vos photos par annee en lisant leurs metadonnees EXIF.

Parcourt recursivement un dossier, detecte la date de chaque photo, puis copie et renomme les fichiers dans une arborescence propre organisee par annee. Inclut une galerie HTML avec lightbox, un systeme de tags/notes, et un export filtre.

```
photos/
  vacances 2008/DCIM/IMG_0001.jpg
  noel/DSC_4521.CR2
  vrac/photo_random.heic

    photo-sort sort photos/

photos_sorted/
  2008/
    2008-07-15_14-30-22.jpg
    .photo_sort_origins
  2019/
    2019-12-25_18-45-01.cr2
    .photo_sort_origins
  2024/
    2024-03-10_09-12-44.heic
    .photo_sort_origins
  .photo_sort_progress.json
  .photo_sort_metadata.json
  gallery.html
```

## Fonctionnalites

- **Detection de date intelligente** -- EXIF (`DateTimeOriginal`, `DateTimeDigitized`, `DateTime`), nom de dossier (regex `19xx`/`20xx`), puis date filesystem en dernier recours
- **Deduplication BLAKE3** -- chaque photo est hashee avant copie, les doublons sont ignores meme s'ils viennent de dossiers differents
- **Reprise apres interruption** -- fichier de progression JSON sauvegarde apres chaque copie, Ctrl+C gere proprement
- **Interface coloree** -- barre de progression, statistiques en temps reel, resume final detaille
- **Dossier de sortie personnalisable** -- possibilite de fusionner plusieurs sources dans un meme dossier de sortie
- **Tracabilite** -- fichier `.photo_sort_origins` dans chaque dossier annee avec la correspondance ancien/nouveau nom
- **14 formats supportes** -- `jpg`, `jpeg`, `heic`, `heif`, `cr2`, `cr3`, `nef`, `arw`, `dng`, `orf`, `rw2`, `raf`, `tiff`, `tif`
- **Galerie HTML** -- grille responsive avec lightbox, diaporama (sequentiel ou aleatoire), navigation clavier
- **Tags et notes** -- systeme de tags libres et notes (1-5) par fichier, persistance JSON
- **Filtres** -- filtrer la galerie et le diaporama par tag et/ou note minimale
- **Export** -- copier les photos correspondant a un filtre vers un dossier de destination

## Installation

```bash
git clone https://github.com/yrbane/photo-sort.git
cd photo-sort
cargo build --release
```

Le binaire se trouve dans `target/release/photo-sort`.

## Utilisation

### Trier les photos

```bash
# Tri basique -- cree un dossier photos_sorted/ a cote
photo-sort sort /chemin/vers/photos

# Dossier de sortie personnalise
photo-sort sort /chemin/vers/photos -o /chemin/vers/sortie

# Fusionner plusieurs sources dans le meme dossier
photo-sort sort /photos/vacances -o /photos/triees
photo-sort sort /photos/noel     -o /photos/triees
photo-sort sort /photos/telephone -o /photos/triees
```

### Taguer et noter

```bash
# Ajouter un tag
photo-sort tag /photos/triees 2008/2008-07-15_14-30-22.jpg vacances

# Retirer un tag
photo-sort tag /photos/triees 2008/2008-07-15_14-30-22.jpg vacances --remove

# Noter un fichier (1-5, 0 pour supprimer)
photo-sort rate /photos/triees 2008/2008-07-15_14-30-22.jpg 5
```

### Generer la galerie HTML

```bash
photo-sort gallery /photos/triees
# Ouvrir gallery.html dans un navigateur
```

La galerie offre :
- Grille responsive groupee par annee
- Lightbox avec navigation clavier (fleches, Echap)
- Diaporama avec vitesse reglable (1-15s), pause, precedent/suivant, mode aleatoire
- Filtres par tag et note minimale (affectent la grille et le diaporama)
- Edition de tags inline (ajout, suppression, suggestions en un clic)
- Notation par etoiles cliquables (1-5, raccourcis clavier 0-5)
- Telechargement individuel de photos

### Galerie interactive (mode serveur)

```bash
photo-sort serve /photos/triees
# Ouvre http://localhost:8080 dans le navigateur

# Port personnalise
photo-sort serve /photos/triees -p 3000
```

Le mode serveur ajoute des fonctionnalites supplementaires :
- **Sauvegarde directe** des tags et notes (sans telecharger de fichier)
- **Suppression** d'une photo avec confirmation
- **Deplacement** d'une photo vers un autre dossier (annee)
- **Rotation** (90/180/270 degres) des images JPEG, PNG, TIFF

### Exporter des fichiers filtres

```bash
# Exporter toutes les photos taguees "vacances"
photo-sort export /photos/triees /export/vacances --tag vacances

# Exporter les photos notees 4 ou plus
photo-sort export /photos/triees /export/meilleures --rating 4

# Combiner tag et note
photo-sort export /photos/triees /export/top-vacances --tag vacances --rating 4
```

## Detection de date

La date de chaque photo est determinee selon cet ordre de priorite :

| Priorite | Methode      | Description                                            | Exemple                              |
| -------- | ------------ | ------------------------------------------------------ | ------------------------------------ |
| 1        | **EXIF**     | Metadonnees EXIF embarquees dans le fichier             | `DateTimeOriginal: 2008-07-15 14:30` |
| 2        | **Dossier**  | Regex `(19\|20)\d{2}` dans le chemin du fichier         | `vacances 2008/DCIM/` -> `2008`      |
| 3        | **Systeme**  | Date de creation ou modification du fichier             | `created: 2024-03-10`               |

## Renommage

Les fichiers sont renommes au format `yyyy-mm-dd_HH-MM-SS.ext`. En cas de collision, un suffixe incremental est ajoute :

```
2008-07-15_14-30-22.jpg
2008-07-15_14-30-22_1.jpg
2008-07-15_14-30-22_2.jpg
```

## Fichiers generes

| Fichier | Emplacement | Description |
| ------- | ----------- | ----------- |
| `.photo_sort_progress.json` | Racine sortie | Progression + correspondance source/destination/hash |
| `.photo_sort_metadata.json` | Racine sortie | Tags et notes par fichier |
| `.photo_sort_origins` | Chaque dossier annee | Correspondance nouveau nom / chemin original |
| `gallery.html` | Racine sortie | Galerie HTML autonome |

## Tests

```bash
cargo test
```

97 tests unitaires couvrant : tri, detection de date, renommage, collisions, hash BLAKE3, progression, metadata (tags/notes), galerie HTML, export filtre, serveur HTTP (API delete/move/rotate/metadata).

## Licence

MIT
