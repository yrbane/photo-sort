# photo-sort

Outil CLI en Rust qui trie automatiquement vos photos par annee en lisant leurs metadonnees EXIF.

Parcourt recursivement un dossier, detecte la date de chaque photo, puis copie et renomme les fichiers dans une arborescence propre organisee par annee.

```
photos/
  vacances 2008/DCIM/IMG_0001.jpg
  noel/DSC_4521.CR2
  vrac/photo_random.heic

    photo-sort photos/

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
```

## Fonctionnalites

- **Detection de date intelligente** -- EXIF (`DateTimeOriginal`, `DateTimeDigitized`, `DateTime`), nom de dossier (regex `19xx`/`20xx`), puis date filesystem en dernier recours
- **Deduplication BLAKE3** -- chaque photo est hashee avant copie, les doublons sont ignores meme s'ils viennent de dossiers differents
- **Reprise apres interruption** -- fichier de progression JSON sauvegarde apres chaque copie, Ctrl+C gere proprement
- **Interface coloree** -- barre de progression, statistiques en temps reel, resume final detaille
- **Dossier de sortie personnalisable** -- possibilite de fusionner plusieurs sources dans un meme dossier de sortie
- **Tracabilite** -- fichier `.photo_sort_origins` dans chaque dossier annee avec la correspondance ancien/nouveau nom
- **14 formats supportes** -- `jpg`, `jpeg`, `heic`, `heif`, `cr2`, `cr3`, `nef`, `arw`, `dng`, `orf`, `rw2`, `raf`, `tiff`, `tif`

## Installation

```bash
git clone https://github.com/yrbane/photo-sort.git
cd photo-sort
cargo build --release
```

Le binaire se trouve dans `target/release/photo-sort`.

## Utilisation

```bash
# Tri basique -- cree un dossier photos_sorted/ a cote
photo-sort /chemin/vers/photos

# Dossier de sortie personnalise
photo-sort /chemin/vers/photos -o /chemin/vers/sortie

# Fusionner plusieurs sources dans le meme dossier
photo-sort /photos/vacances -o /photos/triees
photo-sort /photos/noel     -o /photos/triees
photo-sort /photos/telephone -o /photos/triees
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

### `.photo_sort_progress.json`

Fichier de progression a la racine du dossier de sortie. Permet la reprise et la tracabilite complete :

```json
{
  "processed": [
    {
      "source": "/photos/vacances 2008/DCIM/IMG_0001.jpg",
      "dest": "2008/2008-07-15_14-30-22.jpg",
      "size": 4523123,
      "hash": "a1b2c3d4e5f6...",
      "date_source": "exif"
    }
  ]
}
```

### `.photo_sort_origins`

Un fichier par dossier annee, listant la correspondance entre le nouveau nom et le chemin original :

```
2008-07-15_14-30-22.jpg <- /photos/vacances 2008/DCIM/IMG_0001.jpg
2008-07-15_14-30-22_1.jpg <- /photos/autre dossier/IMG_0002.jpg
```

## Tests

```bash
cargo test
```

26 tests unitaires couvrant la detection de date, le renommage, la gestion des collisions, le hash BLAKE3, la serialisation du fichier de progression et la tracabilite.

## Licence

MIT
