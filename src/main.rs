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
    let tag = mp4ameta::Tag::read_from_path(path)?;
    let artwork = tag
        .artwork()
        .ok_or_else(|| anyhow!("No artwork found in M4A"))?;
    let mime = match artwork.fmt {
        mp4ameta::ImgFmt::Png => "image/png",
        mp4ameta::ImgFmt::Bmp => "image/bmp",
        _ => "image/jpeg",
    }.to_string();
    Ok((artwork.data.to_vec(), mime))
}

fn write_m4a_artwork(audio_path: &PathBuf, image_path: &PathBuf) -> Result<()> {
    let image_data = fs::read(image_path)?;
    let mime = mime_guess::from_path(image_path)
        .first_or_octet_stream()
        .to_string();

    let img = match mime.as_str() {
        "image/png" => mp4ameta::Img::png(image_data),
        "image/bmp" => mp4ameta::Img::bmp(image_data),
        _ => mp4ameta::Img::jpeg(image_data),
    };

    let mut tag = mp4ameta::Tag::read_from_path(audio_path)?;
    tag.set_artwork(img);
    tag.write_to_path(audio_path)?;
    Ok(())
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
