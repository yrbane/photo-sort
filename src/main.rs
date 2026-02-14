mod export;
mod gallery;
mod metadata;
mod serve;
mod sort;
mod thumb;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use std::fs;
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "photo-sort", about = "Trie les photos par année selon leurs métadonnées EXIF")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Trier les photos par année
    Sort {
        /// Chemin du dossier source contenant les photos
        source: PathBuf,
        /// Dossier de sortie (par défaut : <source>_sorted/)
        #[arg(short, long)]
        output: Option<PathBuf>,
    },
    /// Ajouter ou retirer un tag sur un fichier
    Tag {
        /// Dossier de sortie (contenant .photo_sort_metadata.json)
        dir: PathBuf,
        /// Chemin relatif du fichier (ex: 2008/2008-07-15_14-30-22.jpg)
        file: String,
        /// Tag à ajouter ou retirer
        tag: String,
        /// Retirer le tag au lieu de l'ajouter
        #[arg(short, long)]
        remove: bool,
    },
    /// Noter un fichier (1-5)
    Rate {
        /// Dossier de sortie (contenant .photo_sort_metadata.json)
        dir: PathBuf,
        /// Chemin relatif du fichier
        file: String,
        /// Note de 1 à 5 (0 pour supprimer)
        rating: u8,
    },
    /// Générer une galerie HTML avec lightbox et diaporama
    Gallery {
        /// Dossier de sortie contenant les photos triées
        dir: PathBuf,
    },
    /// Lancer la galerie dans le navigateur avec serveur local
    Serve {
        /// Dossier contenant les photos triées
        dir: PathBuf,
        /// Port du serveur (par défaut : 8080)
        #[arg(short, long, default_value_t = 8080)]
        port: u16,
    },
    /// Exporter les fichiers correspondant à un filtre
    Export {
        /// Dossier de sortie contenant les photos triées
        dir: PathBuf,
        /// Dossier de destination pour l'export
        dest: PathBuf,
        /// Filtrer par tag
        #[arg(short, long)]
        tag: Option<String>,
        /// Filtrer par note minimale (1-5)
        #[arg(short, long)]
        rating: Option<u8>,
    },
}

fn resolve_output_dir(source: &std::path::Path, output: Option<PathBuf>) -> Result<PathBuf> {
    let source = source
        .canonicalize()
        .with_context(|| format!("Dossier source introuvable : {}", source.display()))?;

    if !source.is_dir() {
        anyhow::bail!("{} n'est pas un dossier", source.display());
    }

    let output_dir = match output {
        Some(p) => fs::canonicalize(&p).unwrap_or(p),
        None => {
            let source_name = source
                .file_name()
                .unwrap_or_default()
                .to_string_lossy();
            source
                .parent()
                .unwrap_or(&source)
                .join(format!("{source_name}_sorted"))
        }
    };

    Ok(output_dir)
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Sort { source, output } => {
            let source = source
                .canonicalize()
                .with_context(|| format!("Dossier source introuvable : {}", source.display()))?;
            let output_dir = resolve_output_dir(&source, output)?;
            sort::run_sort(&source, &output_dir)
        }
        Commands::Tag {
            dir,
            file,
            tag,
            remove,
        } => {
            let mut meta = metadata::Metadata::load(&dir)?;
            if remove {
                meta.remove_tag(&file, &tag);
                println!("Tag «{tag}» retiré de {file}");
            } else {
                meta.add_tag(&file, &tag);
                println!("Tag «{tag}» ajouté à {file}");
            }
            meta.save(&dir)
        }
        Commands::Rate { dir, file, rating } => {
            if rating > 5 {
                anyhow::bail!("La note doit être entre 0 et 5");
            }
            let mut meta = metadata::Metadata::load(&dir)?;
            if rating == 0 {
                meta.set_rating(&file, None);
                println!("Note supprimée pour {file}");
            } else {
                meta.set_rating(&file, Some(rating));
                println!("Note {rating}/5 attribuée à {file}");
            }
            meta.save(&dir)
        }
        Commands::Gallery { dir } => gallery::run_gallery(&dir),
        Commands::Serve { dir, port } => serve::run_serve(&dir, port),
        Commands::Export {
            dir,
            dest,
            tag,
            rating,
        } => export::run_export(&dir, &dest, tag.as_deref(), rating),
    }
}
