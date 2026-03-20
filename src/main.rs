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

    let moov_data = &data[moov_start..moov_start + moov_size];

    // Try to find ilst within moov/meta or directly in moov/udta
    if let Some((ilst_rel_start, ilst_size)) = find_atom_pos(moov_data, b"ilst") {
        // ilst exists, update it
        let ilst_start = moov_start + ilst_rel_start;
        let ilst_data = &data[ilst_start..ilst_start + ilst_size];
        let mut new_ilst_data = Vec::new();
        new_ilst_data.extend_from_slice(&data[ilst_start..ilst_start + 8]);

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
        new_ilst_data.extend_from_slice(covr_atom);
        let new_ilst_size = new_ilst_data.len() as u32;
        new_ilst_data[0..4].copy_from_slice(&new_ilst_size.to_be_bytes());

        let size_diff = new_ilst_data.len() as i64 - ilst_size as i64;
        rebuild_m4a_with_size_diff(data, moov_start, moov_size, ilst_rel_start, &new_ilst_data, size_diff, output_path)
    } else {
        // No ilst found, create meta/udta/ilst structure
        create_meta_ilst(data, moov_start, moov_size, covr_atom, output_path)
    }
}

fn create_meta_ilst(
    data: &[u8],
    moov_start: usize,
    moov_size: usize,
    covr_atom: &[u8],
    output_path: &PathBuf,
) -> Result<()> {
    // Build ilst atom
    let ilst_size = 8 + covr_atom.len();
    let mut ilst = Vec::with_capacity(ilst_size);
    ilst.extend_from_slice(&(ilst_size as u32).to_be_bytes());
    ilst.extend_from_slice(b"ilst");
    ilst.extend_from_slice(covr_atom);

    // Build hdlr atom (handler for metadata)
    let hdlr_size = 33u32;
    let mut hdlr = Vec::with_capacity(hdlr_size as usize);
    hdlr.extend_from_slice(&hdlr_size.to_be_bytes());
    hdlr.extend_from_slice(b"hdlr");
    hdlr.extend_from_slice(&0u32.to_be_bytes()); // version + flags
    hdlr.extend_from_slice(&0u32.to_be_bytes()); // pre_defined
    hdlr.extend_from_slice(b"mdir"); // handler_type
    hdlr.extend_from_slice(&0u32.to_be_bytes()); // reserved
    hdlr.extend_from_slice(&0u32.to_be_bytes()); // reserved
    hdlr.extend_from_slice(&0u32.to_be_bytes()); // reserved
    hdlr.extend_from_slice(b"\x00"); // name (null terminated)

    // Build meta atom (contains hdlr + ilst)
    let meta_content_size = hdlr.len() + ilst.len();
    let meta_size = 12 + meta_content_size; // meta uses 12-byte header (size + "meta" + version/flags)
    let mut meta = Vec::with_capacity(meta_size);
    meta.extend_from_slice(&(meta_size as u32).to_be_bytes());
    meta.extend_from_slice(b"meta");
    meta.extend_from_slice(&0u32.to_be_bytes()); // version + flags
    meta.extend_from_slice(&hdlr);
    meta.extend_from_slice(&ilst);

    // Try to find udta within moov, or create it
    let moov_data = &data[moov_start..moov_start + moov_size];

    fn find_atom_pos(data: &[u8], name: &[u8; 4]) -> Option<(usize, usize)> {
        let mut i = 0;
        while i + 8 <= data.len() {
            let size = u32::from_be_bytes([data[i], data[i + 1], data[i + 2], data[i + 3]])
                as usize;
            if size < 8 || i + size > data.len() {
                break;
            }
            if &data[i + 4..i + 8] == name {
                return Some((i, size));
            }
            i += size;
        }
        None
    }

    if let Some((udta_rel_start, udta_size)) = find_atom_pos(moov_data, b"udta") {
        // udta exists, insert meta into it
        let udta_start = moov_start + udta_rel_start;
        let udta_data = &data[udta_start..udta_start + udta_size];

        let mut new_udta = Vec::new();
        new_udta.extend_from_slice(&udta_data[..8]); // udta header

        // Copy existing children
        let mut pos = 8;
        while pos < udta_size {
            let size = u32::from_be_bytes([
                udta_data[pos],
                udta_data[pos + 1],
                udta_data[pos + 2],
                udta_data[pos + 3],
            ]) as usize;
            if size < 8 {
                break;
            }
            new_udta.extend_from_slice(&udta_data[pos..pos + size]);
            pos += size;
        }

        // Add meta
        new_udta.extend_from_slice(&meta);

        // Update udta size
        let new_udta_size = new_udta.len() as u32;
        new_udta[0..4].copy_from_slice(&new_udta_size.to_be_bytes());

        let size_diff = new_udta.len() as i64 - udta_size as i64;
        rebuild_m4a_with_size_diff(data, moov_start, moov_size, udta_rel_start, &new_udta, size_diff, output_path)
    } else {
        // No udta, create it and add to end of moov
        let udta_size = 8 + meta.len();
        let mut udta = Vec::with_capacity(udta_size);
        udta.extend_from_slice(&(udta_size as u32).to_be_bytes());
        udta.extend_from_slice(b"udta");
        udta.extend_from_slice(&meta);

        let size_diff = udta.len() as i64;

        // Build new moov
        let new_moov_size = (moov_size as u64 + size_diff as u64) as u32;
        let mut new_data = Vec::with_capacity(data.len() + size_diff as usize);

        // Copy ftyp and everything before moov
        new_data.extend_from_slice(&data[..moov_start]);

        // Write new moov header
        new_data.extend_from_slice(&new_moov_size.to_be_bytes());
        new_data.extend_from_slice(b"moov");

        // Copy moov content
        new_data.extend_from_slice(&data[moov_start + 8..moov_start + moov_size]);

        // Add udta at end of moov
        new_data.extend_from_slice(&udta);

        // Copy everything after moov
        new_data.extend_from_slice(&data[moov_start + moov_size..]);

        // Update stco/co64 offsets
        update_chunk_offsets(&mut new_data, size_diff);

        fs::write(output_path, &new_data)?;
        Ok(())
    }
}

fn rebuild_m4a_with_size_diff(
    data: &[u8],
    moov_start: usize,
    moov_size: usize,
    replace_rel_start: usize,
    replace_data: &[u8],
    size_diff: i64,
    output_path: &PathBuf,
) -> Result<()> {
    let mut new_data = Vec::with_capacity((data.len() as i64 + size_diff) as usize);

    // Copy before moov
    new_data.extend_from_slice(&data[..moov_start]);

    // Build new moov
    let new_moov_size = (moov_size as i64 + size_diff) as u32;
    new_data.extend_from_slice(&new_moov_size.to_be_bytes());
    new_data.extend_from_slice(b"moov");

    let moov_data = &data[moov_start + 8..moov_start + moov_size];
    let replace_start = replace_rel_start - 8; // adjust for moov header

    // Copy moov content before replace position
    new_data.extend_from_slice(&moov_data[..replace_start]);

    // Insert new data
    new_data.extend_from_slice(replace_data);

    // Copy rest of moov content
    new_data.extend_from_slice(&moov_data[replace_start + (replace_data.len() as i64 - size_diff) as usize..]);

    // Copy after moov
    new_data.extend_from_slice(&data[moov_start + moov_size..]);

    // Update stco/co64 offsets
    update_chunk_offsets(&mut new_data, size_diff);

    fs::write(output_path, &new_data)?;
    Ok(())
}

fn update_chunk_offsets(data: &mut Vec<u8>, size_diff: i64) {
    // Find stco or co64 in moov and update offsets
    fn find_atom_in_data(data: &[u8], name: &[u8; 4]) -> Option<(usize, usize)> {
        let mut i = 0;
        while i + 8 <= data.len() {
            let size = u32::from_be_bytes([data[i], data[i + 1], data[i + 2], data[i + 3]]) as usize;
            if size < 8 || i + size > data.len() {
                i += 1;
                continue;
            }
            if &data[i + 4..i + 8] == name {
                return Some((i, size));
            }
            // Search inside container atoms
            if &data[i + 4..i + 8] == b"moov"
                || &data[i + 4..i + 8] == b"trak"
                || &data[i + 4..i + 8] == b"mdia"
                || &data[i + 4..i + 8] == b"minf"
                || &data[i + 4..i + 8] == b"stbl"
            {
                if let Some(inner) = find_atom_in_data(&data[i + 8..i + size - 8], name) {
                    return Some((i + 8 + inner.0, inner.1));
                }
            }
            i += size;
        }
        None
    }

    // Find moov position
    if let Some((moov_start, moov_size)) = find_atom_in_data(data, b"moov") {
        // Look for stco inside moov
        let moov_data = &data[moov_start..moov_start + moov_size];

        fn find_stco_in_atom(data: &[u8]) -> Option<(usize, usize)> {
            let mut i = 0;
            while i + 8 <= data.len() {
                let size = u32::from_be_bytes([data[i], data[i + 1], data[i + 2], data[i + 3]]) as usize;
                if size < 8 || i + size > data.len() {
                    break;
                }
                if &data[i + 4..i + 8] == b"stco" || &data[i + 4..i + 8] == b"co64" {
                    return Some((i, size));
                }
                // Search inside container atoms
                if &data[i + 4..i + 8] == b"trak"
                    || &data[i + 4..i + 8] == b"mdia"
                    || &data[i + 4..i + 8] == b"minf"
                    || &data[i + 4..i + 8] == b"stbl"
                {
                    if let Some(inner) = find_stco_in_atom(&data[i + 8..i + size]) {
                        return Some((i + 8 + inner.0, inner.1));
                    }
                }
                i += size;
            }
            None
        }

        // Process all stco/co64 atoms
        let mut offset_accum = 0i64;
        loop {
            let search_start = if offset_accum == 0 { 0 } else { (moov_size as i64 + offset_accum - size_diff) as usize };
            let search_data = &data[moov_start..moov_start + moov_size];

            if let Some((stco_rel_pos, stco_size)) = find_stco_in_atom(&search_data[search_start.min(moov_size - 8)..]) {
                let stco_pos = moov_start + search_start.min(moov_size - 8) + stco_rel_pos;
                let stco_data = &data[stco_pos..stco_pos + stco_size];

                if &stco_data[4..8] == b"stco" && stco_size > 16 {
                    let entry_count = u32::from_be_bytes([
                        stco_data[12], stco_data[13], stco_data[14], stco_data[15],
                    ]) as usize;

                    for j in 0..entry_count {
                        let offset_pos = stco_pos + 16 + j * 4;
                        if offset_pos + 4 <= data.len() {
                            let old_offset = u32::from_be_bytes([
                                data[offset_pos], data[offset_pos + 1], data[offset_pos + 2], data[offset_pos + 3],
                            ]);
                            let new_offset = (old_offset as i64 + size_diff) as u32;
                            data[offset_pos..offset_pos + 4].copy_from_slice(&new_offset.to_be_bytes());
                        }
                    }
                    offset_accum += 1;
                } else if &stco_data[4..8] == b"co64" && stco_size > 16 {
                    let entry_count = u32::from_be_bytes([
                        stco_data[12], stco_data[13], stco_data[14], stco_data[15],
                    ]) as usize;

                    for j in 0..entry_count {
                        let offset_pos = stco_pos + 16 + j * 8;
                        if offset_pos + 8 <= data.len() {
                            let old_offset = u64::from_be_bytes([
                                data[offset_pos], data[offset_pos + 1], data[offset_pos + 2], data[offset_pos + 3],
                                data[offset_pos + 4], data[offset_pos + 5], data[offset_pos + 6], data[offset_pos + 7],
                            ]);
                            let new_offset = (old_offset as i64 + size_diff) as u64;
                            data[offset_pos..offset_pos + 8].copy_from_slice(&new_offset.to_be_bytes());
                        }
                    }
                    offset_accum += 1;
                } else {
                    break;
                }
            } else {
                break;
            }

            if offset_accum > 100 {
                break; // Safety limit
            }
        }
    }
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
