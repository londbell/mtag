use anyhow::{anyhow, Result};
use clap::{Parser, Subcommand};
use id3::TagLike;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Parser)]
#[command(name = "mtag")]
#[command(about = "Audio file artwork extractor and writer", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Extract artwork from audio file
    Extract {
        /// Input audio file (mp3, m4a, flac)
        #[arg(short, long)]
        input: PathBuf,
        /// Output image file (optional, defaults to input basename + .jpg/.png)
        #[arg(short, long)]
        output: Option<PathBuf>,
    },
    /// Write artwork to audio file
    Write {
        /// Input audio file (mp3, m4a, flac)
        #[arg(short, long)]
        input: PathBuf,
        /// Image file to embed
        #[arg(short, long)]
        image: PathBuf,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Commands::Extract { input, output } => extract_artwork(&input, output.as_deref()),
        Commands::Write { input, image } => write_artwork(&input, &image),
    }
}

fn extract_artwork(input: &PathBuf, output: Option<&Path>) -> Result<()> {
    let ext = input
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_lowercase())
        .ok_or_else(|| anyhow!("Cannot determine file extension"))?;

    let (data, mime) = match ext.as_str() {
        "mp3" => extract_mp3_artwork(input)?,
        "m4a" | "mp4" => extract_m4a_artwork(input)?,
        "flac" => extract_flac_artwork(input)?,
        _ => return Err(anyhow!("Unsupported format: {}", ext)),
    };

    let output_path = match output {
        Some(p) => p.to_path_buf(),
        None => {
            let stem = input.file_stem().unwrap().to_str().unwrap();
            let ext = if mime.contains("png") { "png" } else { "jpg" };
            input.parent().unwrap().join(format!("{}.{}", stem, ext))
        }
    };

    fs::write(&output_path, &data)?;
    println!("Artwork extracted to: {}", output_path.display());
    Ok(())
}

fn write_artwork(input: &PathBuf, image: &PathBuf) -> Result<()> {
    let ext = input
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_lowercase())
        .ok_or_else(|| anyhow!("Cannot determine file extension"))?;

    match ext.as_str() {
        "mp3" => write_mp3_artwork(input, image)?,
        "m4a" | "mp4" => write_m4a_artwork(input, image)?,
        "flac" => write_flac_artwork(input, image)?,
        _ => return Err(anyhow!("Unsupported format: {}", ext)),
    };

    println!("Artwork written to: {}", input.display());
    Ok(())
}

// ===================== MP3 =====================

fn extract_mp3_artwork(path: &PathBuf) -> Result<(Vec<u8>, String)> {
    let tag = id3::Tag::read_from_path(path)?;
    let picture = tag
        .pictures()
        .next()
        .ok_or_else(|| anyhow!("No artwork found in MP3"))?;
    Ok((picture.data.clone(), picture.mime_type.clone()))
}

fn write_mp3_artwork(audio_path: &PathBuf, image_path: &PathBuf) -> Result<()> {
    let image_data = fs::read(image_path)?;
    let mime = mime_guess::from_path(image_path)
        .first_or_octet_stream()
        .to_string();

    let mut tag = id3::Tag::read_from_path(audio_path).unwrap_or_default();
    let picture = id3::frame::Picture {
        mime_type: mime,
        picture_type: id3::frame::PictureType::CoverFront,
        description: String::new(),
        data: image_data,
    };
    tag.add_frame(picture);
    tag.write_to_path(audio_path, id3::Version::Id3v24)?;
    Ok(())
}

// ===================== M4A =====================

fn extract_m4a_artwork(path: &PathBuf) -> Result<(Vec<u8>, String)> {
    let data = fs::read(path)?;
    // mp4parse doesn't directly expose artwork, so we parse manually
    parse_m4a_artwork_manual(&data)
}

fn parse_m4a_artwork_manual(data: &[u8]) -> Result<(Vec<u8>, String)> {
    // Search for 'covr' atom which contains artwork
    fn find_atom(data: &[u8], name: &[u8; 4]) -> Option<(usize, usize)> {
        let mut i = 0;
        while i + 8 <= data.len() {
            let size = u32::from_be_bytes([data[i], data[i + 1], data[i + 2], data[i + 3]])
                as usize;
            if size < 8 || i + size > data.len() {
                i += 1;
                continue;
            }
            if &data[i + 4..i + 8] == name {
                return Some((i, size));
            }
            i += 1;
        }
        None
    }

    // Find covr atom
    if let Some((start, size)) = find_atom(data, b"covr") {
        let atom_data = &data[start + 8..start + size];
        // covr atom contains data atom
        if let Some((data_start, data_size)) = find_atom(atom_data, b"data") {
            let artwork = &atom_data[data_start + 16..data_start + data_size];
            // Determine mime type from data
            let mime = if artwork.len() > 4 && artwork[0..4] == [0x89, 0x50, 0x4e, 0x47] {
                "image/png".to_string()
            } else {
                "image/jpeg".to_string()
            };
            return Ok((artwork.to_vec(), mime));
        }
    }

    Err(anyhow!("No artwork found in M4A"))
}

fn write_m4a_artwork(audio_path: &PathBuf, image_path: &PathBuf) -> Result<()> {
    let image_data = fs::read(image_path)?;
    let mime = mime_guess::from_path(image_path)
        .first_or_octet_stream()
        .to_string();

    let audio_data = fs::read(audio_path)?;

    // Build covr atom
    let mime_code = if mime.contains("png") {
        14u32 // PNG
    } else {
        13u32 // JPEG
    };

    // data atom: size(4) + "data"(4) + flags(4) + mime_code(4) + data
    let data_atom_size = 8 + 4 + 4 + image_data.len();
    let mut data_atom = Vec::with_capacity(data_atom_size);
    data_atom.extend_from_slice(&(data_atom_size as u32).to_be_bytes());
    data_atom.extend_from_slice(b"data");
    data_atom.extend_from_slice(&0u32.to_be_bytes()); // flags
    data_atom.extend_from_slice(&mime_code.to_be_bytes());
    data_atom.extend_from_slice(&image_data);

    // covr atom: size(4) + "covr"(4) + data_atom
    let covr_size = 8 + data_atom.len();
    let mut covr_atom = Vec::with_capacity(covr_size);
    covr_atom.extend_from_slice(&(covr_size as u32).to_be_bytes());
    covr_atom.extend_from_slice(b"covr");
    covr_atom.extend_from_slice(&data_atom);

    // Find ilst atom or create it
    let result = insert_covr_into_ilst(&audio_data, &covr_atom, audio_path)?;
    Ok(result)
}

fn insert_covr_into_ilst(data: &[u8], covr_atom: &[u8], output_path: &PathBuf) -> Result<()> {
    fn find_atom_pos(data: &[u8], name: &[u8; 4]) -> Option<(usize, usize)> {
        let mut i = 0;
        while i + 8 <= data.len() {
            let size = u32::from_be_bytes([data[i], data[i + 1], data[i + 2], data[i + 3]])
                as usize;
            if size < 8 {
                break;
            }
            if i + size > data.len() {
                break;
            }
            if &data[i + 4..i + 8] == name {
                return Some((i, size));
            }
            i += size;
        }
        None
    }

    // Find moov
    let (moov_start, moov_size) =
        find_atom_pos(data, b"moov").ok_or_else(|| anyhow!("moov atom not found"))?;

    // Find ilst within moov
    let moov_data = &data[moov_start..moov_start + moov_size];
    if let Some((ilst_rel_start, ilst_size)) = find_atom_pos(moov_data, b"ilst") {
        let ilst_start = moov_start + ilst_rel_start;
        let ilst_end = ilst_start + ilst_size;

        // Check if covr already exists and remove it
        let ilst_data = &data[ilst_start..ilst_end];
        let mut new_ilst_data = Vec::new();
        new_ilst_data.extend_from_slice(&data[ilst_start..ilst_start + 8]); // ilst header

        // Copy existing atoms except covr
        let mut pos = 8;
        while pos < ilst_size {
            let size =
                u32::from_be_bytes([ilst_data[pos], ilst_data[pos + 1], ilst_data[pos + 2], ilst_data[pos + 3]])
                    as usize;
            if size < 8 {
                break;
            }
            if &ilst_data[pos + 4..pos + 8] != b"covr" {
                new_ilst_data.extend_from_slice(&ilst_data[pos..pos + size]);
            }
            pos += size;
        }

        // Add new covr
        new_ilst_data.extend_from_slice(covr_atom);

        // Update ilst size
        let new_ilst_size = new_ilst_data.len() as u32;
        new_ilst_data[0..4].copy_from_slice(&new_ilst_size.to_be_bytes());

        // Update moov size
        let size_diff = new_ilst_data.len() as i64 - ilst_size as i64;

        // Build new file
        let mut new_data = Vec::with_capacity(data.len() + size_diff as usize);
        new_data.extend_from_slice(&data[..moov_start]);
        let moov_atom = &data[moov_start..moov_start + moov_size];
        new_data.extend_from_slice(&moov_atom[..8]); // moov header
        let mut moov_pos = 8;
        while moov_pos < moov_size {
            let size = u32::from_be_bytes([
                moov_atom[moov_pos],
                moov_atom[moov_pos + 1],
                moov_atom[moov_pos + 2],
                moov_atom[moov_pos + 3],
            ]) as usize;
            if size < 8 {
                break;
            }
            if moov_pos == ilst_rel_start {
                new_data.extend_from_slice(&new_ilst_data);
            } else {
                new_data.extend_from_slice(&moov_atom[moov_pos..moov_pos + size]);
            }
            moov_pos += size;
        }

        // Update moov size in header
        let new_moov_size = (moov_size as i64 + size_diff) as u32;
        new_data[moov_start..moov_start + 4].copy_from_slice(&new_moov_size.to_be_bytes());

        // Update stco/co64 offsets if needed (simplified - may not work for all files)
        fs::write(output_path, &new_data)?;
        return Ok(());
    }

    // No ilst found - need to create meta/ilst structure (complex, not implemented)
    Err(anyhow!("No ilst atom found, cannot add artwork to this M4A file"))
}

// ===================== FLAC =====================

fn extract_flac_artwork(path: &PathBuf) -> Result<(Vec<u8>, String)> {
    let tag = metaflac::Tag::read_from_path(path)?;
    let picture = tag
        .pictures()
        .next()
        .ok_or_else(|| anyhow!("No artwork found in FLAC"))?;
    Ok((picture.data.clone(), picture.mime_type.clone()))
}

fn write_flac_artwork(audio_path: &PathBuf, image_path: &PathBuf) -> Result<()> {
    let image_data = fs::read(image_path)?;
    let mime = mime_guess::from_path(image_path)
        .first_or_octet_stream()
        .to_string();

    let mut tag = metaflac::Tag::read_from_path(audio_path)?;
    tag.remove_picture_type(metaflac::block::PictureType::CoverFront);
    tag.add_picture(
        mime,
        metaflac::block::PictureType::CoverFront,
        image_data,
    );
    tag.write_to_path(audio_path)?;
    Ok(())
}
