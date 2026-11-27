use serde::Serialize;
use sha2::{Digest, Sha256};
use std::fs::File;
use std::io::{self, Read, Write};
use std::path::{Component, Path, PathBuf};

#[derive(Serialize)]
pub struct EntryInfo {
    name: String,
    size: u64,
    is_dir: bool,
}

#[derive(Serialize)]
pub struct ExtractedInfo {
    path: String,
    size: u64,
    sha256: String,
}

const FOLDER_THEME_ICON: &[u8] = include_bytes!("../icons/folder-silver-graphite.ico");

fn folder_theme_icon_path() -> PathBuf {
    let dir = std::env::current_exe().ok().and_then(|p| p.parent().map(|d| d.to_path_buf())).unwrap_or_else(|| PathBuf::from("."));
    let path = dir.join("folder-silver-graphite.ico");
    let needs_write = std::fs::metadata(&path).map(|m| m.len() != FOLDER_THEME_ICON.len() as u64).unwrap_or(true);
    if needs_write {
        let _ = std::fs::write(&path, FOLDER_THEME_ICON);
    }
    path
}

fn refresh_explorer_icons() {
    #[cfg(windows)]
    {
        let _ = std::process::Command::new("ie4uinit.exe").arg("-show").output();
        // Automatic Explorer update: clear the icon cache and restart
        // Explorer so every folder picks up the new icon at once.
        let _ = std::process::Command::new("taskkill").args(["/f", "/im", "explorer.exe"]).output();
        if let Ok(local) = std::env::var("LOCALAPPDATA") {
            let cache_dir = Path::new(&local).join("Microsoft\\Windows\\Explorer");
            if let Ok(read) = std::fs::read_dir(&cache_dir) {
                for entry in read.flatten() {
                    let name = entry.file_name().to_string_lossy().to_lowercase();
                    if name.starts_with("iconcache") || name.starts_with("thumbcache") {
                        let _ = std::fs::remove_file(entry.path());
                    }
                }
            }
        }
        let _ = std::process::Command::new("explorer.exe").spawn();
    }
}

fn apply_folder_theme(_folder: &Path) -> Result<(), String> {
    // System-wide: replace the default Windows folder icon for the user.
    let icon = folder_theme_icon_path();
    #[cfg(windows)]
    {
        let icon_value = format!("{},0", icon.to_string_lossy());
        reg_add("HKCU\\Software\\Microsoft\\Windows\\CurrentVersion\\Explorer\\Shell Icons", Some("3"), &icon_value);
        reg_add("HKCU\\Software\\Microsoft\\Windows\\CurrentVersion\\Explorer\\Shell Icons", Some("4"), &icon_value);
        reg_add("HKCU\\Software\\Classes\\CLSID\\{86ca1aa0-34aa-4e8b-a509-50c905bae2a2}\\DefaultIcon", None, &icon_value);
        reg_add("HKCU\\Software\\Classes\\Directory\\DefaultIcon", None, &icon_value);
        reg_add("HKCU\\Software\\Classes\\Folder\\DefaultIcon", None, &icon_value);
        refresh_explorer_icons();
    }
    Ok(())
}

fn restore_default_folder_theme() -> Result<(), String> {
    // Remove exactly the values apply_folder_theme writes, so the
    // Windows default folder icon returns.
    #[cfg(windows)]
    {
        reg_delete_value("HKCU\\Software\\Microsoft\\Windows\\CurrentVersion\\Explorer\\Shell Icons", "3");
        reg_delete_value("HKCU\\Software\\Microsoft\\Windows\\CurrentVersion\\Explorer\\Shell Icons", "4");
        reg_delete_key("HKCU\\Software\\Classes\\CLSID\\{86ca1aa0-34aa-4e8b-a509-50c905bae2a2}\\DefaultIcon");
        reg_delete_key("HKCU\\Software\\Classes\\Directory\\DefaultIcon");
        reg_delete_key("HKCU\\Software\\Classes\\Folder\\DefaultIcon");
        refresh_explorer_icons();
    }
    Ok(())
}

fn safe_join(dest: &Path, entry: &str) -> Result<PathBuf, String> {
    let normalized = entry.replace('\\', "/");
    let rel = Path::new(&normalized);
    let mut out = dest.to_path_buf();
    for comp in rel.components() {
        match comp {
            Component::Normal(part) => out.push(part),
            Component::CurDir => {}
            _ => return Err(format!("Unsafe archive entry blocked: {entry}")),
        }
    }
    Ok(out)
}

fn sha256_hex(path: &Path) -> Result<String, String> {
    let mut file = File::open(path).map_err(|e| e.to_string())?;
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 8192];
    loop {
        let n = file.read(&mut buf).map_err(|e| e.to_string())?;
        if n == 0 { break; }
        hasher.update(&buf[..n]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

fn write_stream(mut reader: impl Read, target: &Path) -> Result<u64, String> {
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let mut out = File::create(target).map_err(|e| e.to_string())?;
    io::copy(&mut reader, &mut out).map_err(|e| e.to_string())
}

fn lower_name(path: &Path) -> String {
    path.file_name().map(|n| n.to_string_lossy().to_lowercase()).unwrap_or_default()
}

fn has_house_magic(path: &Path) -> bool {
    if let Ok(mut f) = File::open(path) {
        let mut magic = [0u8; 8];
        if f.read(&mut magic).is_ok() {
            return magic.starts_with(b"ELITE1");
        }
    }
    false
}

fn normalize_custom_extension(input: &str) -> Result<String, String> {
    let ext = input.trim().trim_start_matches('.').to_lowercase();
    if ext.is_empty() || ext.len() > 32 {
        return Err("Custom extension must be 1 to 32 characters".to_string());
    }
    let mut chars = ext.chars();
    let first = chars.next().unwrap();
    if !first.is_ascii_alphanumeric() {
        return Err("Custom extension must start with a letter or number".to_string());
    }
    if !chars.all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '_' || c == '-') {
        return Err("Custom extension may only contain letters, numbers, dot, underscore, or hyphen".to_string());
    }
    const RESERVED: [&str; 19] = ["zip", "7z", "rar", "tar", "gz", "bz2", "xz", "zst", "tgz", "tbz2", "txz", "illuminati", "elite", "jar", "war", "epub", "docx", "xlsx", "pptx"];
    if RESERVED.contains(&ext.as_str()) {
        return Err("Custom extension is reserved for a built-in format".to_string());
    }
    Ok(ext)
}

#[derive(Clone, Copy, PartialEq)]
enum Kind { Zip, SevenZ, Rar, Tar, TarGz, TarBz2, TarXz, TarZst, Gz, Bz2, Xz, Zst, Illuminati }

fn detect(path: &Path) -> Result<Kind, String> {
    if has_house_magic(path) { return Ok(Kind::Illuminati); }
    let name = lower_name(path);
    if name.ends_with(".tar.gz") || name.ends_with(".tgz") { return Ok(Kind::TarGz); }
    if name.ends_with(".tar.bz2") || name.ends_with(".tbz2") { return Ok(Kind::TarBz2); }
    if name.ends_with(".tar.xz") || name.ends_with(".txz") { return Ok(Kind::TarXz); }
    if name.ends_with(".tar.zst") { return Ok(Kind::TarZst); }
    if name.ends_with(".zip") || name.ends_with(".jar") || name.ends_with(".war") || name.ends_with(".epub") || name.ends_with(".docx") || name.ends_with(".xlsx") || name.ends_with(".pptx") { return Ok(Kind::Zip); }
    if name.ends_with(".7z") { return Ok(Kind::SevenZ); }
    if name.ends_with(".rar") { return Ok(Kind::Rar); }
    if name.ends_with(".tar") { return Ok(Kind::Tar); }
    if name.ends_with(".gz") { return Ok(Kind::Gz); }
    if name.ends_with(".bz2") { return Ok(Kind::Bz2); }
    if name.ends_with(".xz") { return Ok(Kind::Xz); }
    if name.ends_with(".zst") { return Ok(Kind::Zst); }
    if name.ends_with(".illuminati") || name.ends_with(".elite") { return Ok(Kind::Illuminati); }
    // Magic-byte fallback
    let mut f = File::open(path).map_err(|e| e.to_string())?;
    let mut magic = [0u8; 8];
    let _ = f.read(&mut magic);
    if magic.starts_with(b"PK\x03\x04") { return Ok(Kind::Zip); }
    if magic.starts_with(b"7z\xbc\xaf\x27\x1c") { return Ok(Kind::SevenZ); }
    if magic.starts_with(b"Rar!") { return Ok(Kind::Rar); }
    if magic.starts_with(&[0x1f, 0x8b]) { return Ok(Kind::Gz); }
    if magic.starts_with(b"BZh") { return Ok(Kind::Bz2); }
    if magic.starts_with(&[0xfd, b'7', b'z', b'X', b'Z', 0x00]) { return Ok(Kind::Xz); }
    if magic.starts_with(&[0x28, 0xb5, 0x2f, 0xfd]) { return Ok(Kind::Zst); }
    if magic.starts_with(b"ELITE1") { return Ok(Kind::Illuminati); }
    Err("Unsupported archive format".to_string())
}

fn tar_reader(path: &Path, kind: Kind) -> Result<Box<dyn Read>, String> {
    let file = File::open(path).map_err(|e| e.to_string())?;
    Ok(match kind {
        Kind::Tar => Box::new(file),
        Kind::TarGz => Box::new(flate2::read::GzDecoder::new(file)),
        Kind::TarBz2 => Box::new(bzip2::read::BzDecoder::new(file)),
        Kind::TarXz => Box::new(xz2::read::XzDecoder::new(file)),
        Kind::TarZst => Box::new(zstd::Decoder::new(file).map_err(|e| e.to_string())?),
        _ => return Err("Not a tar archive".to_string()),
    })
}

fn hex_encode(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn hex_decode(text: &str) -> Result<Vec<u8>, String> {
    if text.len() % 2 != 0 { return Err("Invalid hex data".to_string()); }
    (0..text.len()).step_by(2).map(|i| u8::from_str_radix(&text[i..i + 2], 16).map_err(|e| e.to_string())).collect()
}

fn collect_files(source: &Path, base: &Path, out: &mut Vec<(PathBuf, PathBuf)>) -> Result<(), String> {
    if source.is_file() {
        let rel = source.file_name().map(PathBuf::from).ok_or("Invalid file name")?;
        out.push((source.to_path_buf(), rel));
        return Ok(());
    }
    let mut stack = vec![source.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let rel_dir = dir.strip_prefix(base).map_err(|e| e.to_string())?.to_path_buf();
        out.push((dir.clone(), rel_dir));
        for entry in std::fs::read_dir(&dir).map_err(|e| e.to_string())? {
            let entry = entry.map_err(|e| e.to_string())?;
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if path.is_file() {
                let rel = path.strip_prefix(base).map_err(|e| e.to_string())?.to_path_buf();
                out.push((path, rel));
            }
        }
    }
    Ok(())
}

fn build_tar_bytes(source: &Path) -> Result<Vec<u8>, String> {
    let base = source.parent().unwrap_or_else(|| Path::new("."));
    let mut files = Vec::new();
    collect_files(source, base, &mut files)?;
    let mut builder = tar::Builder::new(Vec::new());
    for (abs, rel) in files {
        builder.append_path_with_name(&abs, &rel).map_err(|e| e.to_string())?;
    }
    builder.into_inner().map_err(|e| e.to_string())
}

fn create_illuminati_archive(source: &Path, dest: &Path, level: i32, password: &str) -> Result<(), String> {
    use aes_gcm::{aead::{Aead, KeyInit}, Aes256Gcm, Key, Nonce};
    use rand::RngCore;
    let tar_bytes = build_tar_bytes(source)?;
    let plain_sha = format!("{:x}", Sha256::digest(&tar_bytes));
    let compressed = zstd::encode_all(&tar_bytes[..], level).map_err(|e| e.to_string())?;
    let encrypted = !password.is_empty();
    let (payload, salt_hex, nonce_hex) = if encrypted {
        let mut salt = [0u8; 16];
        let mut nonce = [0u8; 12];
        rand::rngs::OsRng.fill_bytes(&mut salt);
        rand::rngs::OsRng.fill_bytes(&mut nonce);
        let mut key = [0u8; 32];
        argon2::Argon2::default().hash_password_into(password.as_bytes(), &salt, &mut key).map_err(|e| e.to_string())?;
        let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(&key));
        let ct = cipher.encrypt(Nonce::from_slice(&nonce), compressed.as_ref()).map_err(|e| e.to_string())?;
        (ct, hex_encode(&salt), hex_encode(&nonce))
    } else {
        (compressed, String::new(), String::new())
    };
    let header = serde_json::json!({
        "format": "Illuminati Archive",
        "version": 1,
        "compression": "zstd",
        "level": level,
        "encrypted": encrypted,
        "salt": salt_hex,
        "nonce": nonce_hex,
        "sha256": plain_sha,
    }).to_string();
    let mut out = Vec::new();
    out.extend_from_slice(b"ELITE1\0\0");
    out.extend_from_slice(&(header.len() as u32).to_le_bytes());
    out.extend_from_slice(header.as_bytes());
    out.extend_from_slice(&payload);
    std::fs::write(dest, out).map_err(|e| e.to_string())
}

fn read_illuminati_tar_bytes(path: &Path, password: Option<&str>) -> Result<Vec<u8>, String> {
    use aes_gcm::{aead::{Aead, KeyInit}, Aes256Gcm, Key, Nonce};
    let data = std::fs::read(path).map_err(|e| e.to_string())?;
    if data.len() < 12 || !data.starts_with(b"ELITE1") { return Err("Invalid Illuminati archive".to_string()); }
    let header_len = u32::from_le_bytes(data[8..12].try_into().map_err(|_| "Invalid Illuminati header")?) as usize;
    if data.len() < 12 + header_len { return Err("Invalid Illuminati header".to_string()); }
    let header: serde_json::Value = serde_json::from_slice(&data[12..12 + header_len]).map_err(|e| e.to_string())?;
    let payload = &data[12 + header_len..];
    let compressed = if header["encrypted"].as_bool().unwrap_or(false) {
        let password = password.filter(|p| !p.is_empty()).ok_or("Password required")?;
        let salt = hex_decode(header["salt"].as_str().unwrap_or_default())?;
        let nonce = hex_decode(header["nonce"].as_str().unwrap_or_default())?;
        let mut key = [0u8; 32];
        argon2::Argon2::default().hash_password_into(password.as_bytes(), &salt, &mut key).map_err(|e| e.to_string())?;
        let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(&key));
        cipher.decrypt(Nonce::from_slice(&nonce), payload).map_err(|_| "Invalid password or corrupted archive".to_string())?
    } else {
        payload.to_vec()
    };
    let tar_bytes = zstd::decode_all(&compressed[..]).map_err(|e| e.to_string())?;
    let expected = header["sha256"].as_str().unwrap_or_default();
    let actual = format!("{:x}", Sha256::digest(&tar_bytes));
    if !expected.is_empty() && expected != actual { return Err("Illuminati archive integrity check failed".to_string()); }
    Ok(tar_bytes)
}

fn illuminati_is_encrypted(path: &Path) -> bool {
    if let Ok(data) = std::fs::read(path) {
        if data.len() >= 12 && data.starts_with(b"ELITE1") {
            let header_len = u32::from_le_bytes(data[8..12].try_into().unwrap_or([0; 4])) as usize;
            if data.len() >= 12 + header_len {
                if let Ok(header) = serde_json::from_slice::<serde_json::Value>(&data[12..12 + header_len]) {
                    return header["encrypted"].as_bool().unwrap_or(false);
                }
            }
        }
    }
    false
}

fn create_zip_archive(source: &Path, dest: &Path, level: i32) -> Result<(), String> {
    let base = source.parent().unwrap_or_else(|| Path::new("."));
    let mut files = Vec::new();
    collect_files(source, base, &mut files)?;
    let file = File::create(dest).map_err(|e| e.to_string())?;
    let mut zip = zip::ZipWriter::new(file);
    let options = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated).compression_level(Some(level.clamp(0, 9) as i64));
    for (abs, rel) in files {
        let name = rel.to_string_lossy().replace('\\', "/");
        if abs.is_dir() {
            zip.add_directory(format!("{name}/"), options).map_err(|e| e.to_string())?;
        } else {
            zip.start_file(name, options).map_err(|e| e.to_string())?;
            let mut input = File::open(&abs).map_err(|e| e.to_string())?;
            io::copy(&mut input, &mut zip).map_err(|e| e.to_string())?;
        }
    }
    zip.finish().map_err(|e| e.to_string())?;
    Ok(())
}

fn list_archive_inner(path: &Path) -> Result<Vec<EntryInfo>, String> {
    let kind = detect(path)?;
    let mut out = Vec::new();
    match kind {
        Kind::Zip => {
            let file = File::open(path).map_err(|e| e.to_string())?;
            let mut zip = zip::ZipArchive::new(file).map_err(|e| e.to_string())?;
            for i in 0..zip.len() {
                let entry = zip.by_index(i).map_err(|e| e.to_string())?;
                out.push(EntryInfo { name: entry.name().to_string(), size: entry.size(), is_dir: entry.is_dir() });
            }
        }
        Kind::SevenZ => {
            let reader = sevenz_rust::SevenZReader::open(path, sevenz_rust::Password::empty()).map_err(|e| e.to_string())?;
            for entry in &reader.archive().files {
                out.push(EntryInfo { name: entry.name.clone(), size: entry.size, is_dir: entry.is_directory });
            }
        }
        Kind::Rar => {
            let archive = unrar::Archive::new(path).open_for_listing().map_err(|e| e.to_string())?;
            for header in archive {
                let header = header.map_err(|e| e.to_string())?;
                out.push(EntryInfo { name: header.filename.to_string_lossy().to_string(), size: header.unpacked_size, is_dir: !header.is_file() });
            }
        }
        Kind::Tar | Kind::TarGz | Kind::TarBz2 | Kind::TarXz | Kind::TarZst => {
            let mut archive = tar::Archive::new(tar_reader(path, kind)?);
            for entry in archive.entries().map_err(|e| e.to_string())? {
                let entry = entry.map_err(|e| e.to_string())?;
                let name = entry.path().map_err(|e| e.to_string())?.to_string_lossy().to_string();
                out.push(EntryInfo { name, size: entry.size(), is_dir: entry.header().entry_type().is_dir() });
            }
        }
        Kind::Gz | Kind::Bz2 | Kind::Xz | Kind::Zst => {
            let stem = path.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
            out.push(EntryInfo { name: stem, size: 0, is_dir: false });
        }
        Kind::Illuminati => {
            let tar_bytes = read_illuminati_tar_bytes(path, None)?;
            let mut archive = tar::Archive::new(io::Cursor::new(tar_bytes));
            for entry in archive.entries().map_err(|e| e.to_string())? {
                let entry = entry.map_err(|e| e.to_string())?;
                let name = entry.path().map_err(|e| e.to_string())?.to_string_lossy().to_string();
                out.push(EntryInfo { name, size: entry.size(), is_dir: entry.header().entry_type().is_dir() });
            }
        }
    }
    Ok(out)
}

fn extract_archive_inner(path: &Path, dest: &Path) -> Result<Vec<ExtractedInfo>, String> {
    extract_archive_inner_with_password(path, dest, None)
}

fn extract_archive_inner_with_password(path: &Path, dest: &Path, password: Option<&str>) -> Result<Vec<ExtractedInfo>, String> {
    let kind = detect(path)?;
    std::fs::create_dir_all(dest).map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    match kind {
        Kind::Zip => {
            let file = File::open(path).map_err(|e| e.to_string())?;
            let mut zip = zip::ZipArchive::new(file).map_err(|e| e.to_string())?;
            for i in 0..zip.len() {
                let mut entry = zip.by_index(i).map_err(|e| e.to_string())?;
                let name = entry.name().to_string();
                let target = safe_join(dest, &name)?;
                if entry.is_dir() {
                    std::fs::create_dir_all(&target).map_err(|e| e.to_string())?;
                } else {
                    let size = write_stream(&mut entry, &target)?;
                    out.push(ExtractedInfo { path: target.to_string_lossy().to_string(), size, sha256: sha256_hex(&target)? });
                }
            }
        }
        Kind::SevenZ => {
            let mut reader = sevenz_rust::SevenZReader::open(path, sevenz_rust::Password::empty()).map_err(|e| e.to_string())?;
            let dest_buf = dest.to_path_buf();
            let mut collected: Vec<ExtractedInfo> = Vec::new();
            reader.for_each_entries(|entry, entry_reader| {
                let target = safe_join(&dest_buf, &entry.name).map_err(|e| sevenz_rust::Error::Other(e.into()))?;
                if entry.is_directory {
                    std::fs::create_dir_all(&target).map_err(|e| sevenz_rust::Error::Other(e.to_string().into()))?;
                } else {
                    let size = write_stream(entry_reader, &target).map_err(|e| sevenz_rust::Error::Other(e.into()))?;
                    let hash = sha256_hex(&target).map_err(|e| sevenz_rust::Error::Other(e.into()))?;
                    collected.push(ExtractedInfo { path: target.to_string_lossy().to_string(), size, sha256: hash });
                }
                Ok(true)
            }).map_err(|e| e.to_string())?;
            out = collected;
        }
        Kind::Rar => {
            let mut archive = unrar::Archive::new(path).open_for_processing().map_err(|e| e.to_string())?;
            while let Some(header) = archive.read_header().map_err(|e| e.to_string())? {
                let entry = header.entry().clone();
                let name = entry.filename.to_string_lossy().to_string();
                let target = safe_join(dest, &name)?;
                if entry.is_file() {
                    archive = header.extract_to(&target).map_err(|e| e.to_string())?;
                    let size = std::fs::metadata(&target).map(|m| m.len()).unwrap_or(0);
                    out.push(ExtractedInfo { path: target.to_string_lossy().to_string(), size, sha256: sha256_hex(&target)? });
                } else {
                    std::fs::create_dir_all(&target).map_err(|e| e.to_string())?;
                    archive = header.skip().map_err(|e| e.to_string())?;
                }
            }
        }
        Kind::Tar | Kind::TarGz | Kind::TarBz2 | Kind::TarXz | Kind::TarZst => {
            let mut archive = tar::Archive::new(tar_reader(path, kind)?);
            for entry in archive.entries().map_err(|e| e.to_string())? {
                let mut entry = entry.map_err(|e| e.to_string())?;
                let name = entry.path().map_err(|e| e.to_string())?.to_string_lossy().to_string();
                let target = safe_join(dest, &name)?;
                if entry.header().entry_type().is_dir() {
                    std::fs::create_dir_all(&target).map_err(|e| e.to_string())?;
                } else {
                    let size = write_stream(&mut entry, &target)?;
                    out.push(ExtractedInfo { path: target.to_string_lossy().to_string(), size, sha256: sha256_hex(&target)? });
                }
            }
        }
        Kind::Gz | Kind::Bz2 | Kind::Xz | Kind::Zst => {
            let stem = path.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| "output".to_string());
            let target = safe_join(dest, &stem)?;
            let file = File::open(path).map_err(|e| e.to_string())?;
            let size = match kind {
                Kind::Gz => write_stream(flate2::read::GzDecoder::new(file), &target)?,
                Kind::Bz2 => write_stream(bzip2::read::BzDecoder::new(file), &target)?,
                Kind::Xz => write_stream(xz2::read::XzDecoder::new(file), &target)?,
                _ => write_stream(zstd::Decoder::new(file).map_err(|e| e.to_string())?, &target)?,
            };
            out.push(ExtractedInfo { path: target.to_string_lossy().to_string(), size, sha256: sha256_hex(&target)? });
        }
        Kind::Illuminati => {
            let tar_bytes = read_illuminati_tar_bytes(path, password)?;
            let mut archive = tar::Archive::new(io::Cursor::new(tar_bytes));
            for entry in archive.entries().map_err(|e| e.to_string())? {
                let mut entry = entry.map_err(|e| e.to_string())?;
                let name = entry.path().map_err(|e| e.to_string())?.to_string_lossy().to_string();
                let target = safe_join(dest, &name)?;
                if entry.header().entry_type().is_dir() {
                    std::fs::create_dir_all(&target).map_err(|e| e.to_string())?;
                } else {
                    let size = write_stream(&mut entry, &target)?;
                    out.push(ExtractedInfo { path: target.to_string_lossy().to_string(), size, sha256: sha256_hex(&target)? });
                }
            }
        }
    }
    Ok(out)
}

#[tauri::command]
fn list_archive(path: String) -> Result<Vec<EntryInfo>, String> {
    list_archive_inner(Path::new(&path))
}

#[tauri::command]
fn extract_archive(path: String, dest: String) -> Result<Vec<ExtractedInfo>, String> {
    extract_archive_inner(Path::new(&path), Path::new(&dest))
}

#[tauri::command]
fn extract_here(path: String) -> Result<Vec<ExtractedInfo>, String> {
    let p = Path::new(&path);
    let dest = p.parent().ok_or("Archive has no parent folder")?;
    extract_archive_inner(p, dest)
}

#[tauri::command]
fn extract_to_folder(path: String) -> Result<Vec<ExtractedInfo>, String> {
    let p = Path::new(&path);
    let parent = p.parent().ok_or("Archive has no parent folder")?;
    let stem = p.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| "extracted".to_string());
    let stem = stem.trim_end_matches(".tar").to_string();
    extract_archive_inner(p, &parent.join(stem))
}

#[derive(Serialize)]
struct DialogContext {
    mode: String,
    source: String,
    parent: String,
    suggested_name: String,
}

#[tauri::command]
fn dialog_context() -> DialogContext {
    let args: Vec<String> = std::env::args().collect();
    let mut mode = "add".to_string();
    let mut source = String::new();
    for pair in args.windows(2) {
        if pair[0] == "--add" {
            mode = "add".to_string();
            source = pair[1].clone();
        } else if pair[0] == "--extract-here" || pair[0] == "--extract-to" {
            mode = "password".to_string();
            source = pair[1].clone();
        }
    }
    let path = Path::new(&source);
    let parent = path.parent().map(|p| p.to_string_lossy().to_string()).unwrap_or_default();
    let suggested_name = path.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| "Archive".to_string());
    DialogContext { mode, source, parent, suggested_name }
}

#[tauri::command]
fn create_archive(source: String, dest: String, format: String, level: i32, password: String, custom_extension: Option<String>, register_custom: bool) -> Result<String, String> {
    let source = Path::new(&source);
    let dest = Path::new(&dest);
    let effective_dest = match format.as_str() {
        "illuminati" | "elite" => {
            let effective = dest.with_extension("illuminati");
            create_illuminati_archive(source, &effective, level, &password)?;
            effective
        }
        "custom" => {
            let ext = normalize_custom_extension(custom_extension.as_deref().unwrap_or_default())?;
            let effective = dest.with_extension(&ext);
            create_illuminati_archive(source, &effective, level, &password)?;
            if register_custom {
                #[cfg(windows)]
                {
                    let exe = std::env::current_exe().map(|p| p.to_string_lossy().to_string()).unwrap_or_default();
                    register_archive_extension(&exe, &format!(".{ext}"));
                    register_house_file_type(&exe, &format!(".{ext}"));
                }
            }
            effective
        }
        "zip" => {
            let effective = dest.with_extension("zip");
            create_zip_archive(source, &effective, level)?;
            effective
        }
        _ => return Err("Unsupported archive format".to_string()),
    };
    let size = std::fs::metadata(&effective_dest).map(|m| m.len()).unwrap_or(0);
    Ok(format!("Created {} ({} bytes)", effective_dest.to_string_lossy(), size))
}

#[tauri::command]
fn extract_with_password(password: String) -> Result<String, String> {
    let args: Vec<String> = std::env::args().collect();
    for pair in args.windows(2) {
        let p = Path::new(&pair[1]);
        if pair[0] == "--extract-here" {
            let parent = p.parent().ok_or("Archive has no parent folder")?;
            let files = extract_archive_inner_with_password(p, parent, Some(&password))?;
            return Ok(format!("Extracted {} files", files.len()));
        }
        if pair[0] == "--extract-to" {
            let parent = p.parent().ok_or("Archive has no parent folder")?;
            let stem = p.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| "extracted".to_string());
            let stem = stem.trim_end_matches(".tar").to_string();
            let files = extract_archive_inner_with_password(p, &parent.join(stem), Some(&password))?;
            return Ok(format!("Extracted {} files", files.len()));
        }
    }
    Err("No extraction request found".to_string())
}

#[tauri::command]
fn initial_path() -> Option<String> {
    let args: Vec<String> = std::env::args().collect();
    for pair in args.windows(2) {
        if pair[0] == "--list" || pair[0] == "--open" {
            return Some(pair[1].clone());
        }
    }
    args.get(1).filter(|a| !a.starts_with('-')).cloned()
}

#[cfg(windows)]
fn reg_add(key: &str, value_name: Option<&str>, data: &str) {
    let mut cmd = std::process::Command::new("reg");
    cmd.args(["add", key]);
    if let Some(name) = value_name {
        cmd.args(["/v", name]);
    } else {
        cmd.arg("/ve");
    }
    cmd.args(["/d", data, "/f"]);
    let _ = cmd.output();
}

fn reg_delete_value(key: &str, value_name: &str) {
    let _ = std::process::Command::new("reg").args(["delete", key, "/v", value_name, "/f"]).output();
}

fn reg_delete_key(key: &str) {
    let _ = std::process::Command::new("reg").args(["delete", key, "/f"]).output();
}

#[cfg(windows)]
fn register_archive_extension(exe: &str, ext: &str) {
    let verbs = [
        ("ExtractHere", "Extract here with A Elite Estrangeira", "--extract-here"),
        ("ExtractToFolder", "Extract to folder with A Elite Estrangeira", "--extract-to"),
        ("ListArchive", "List with A Elite Estrangeira", "--list"),
    ];
    for legacy in ["EliteExtratorHere", "EliteExtratorList", "EliteExtratorToFolder", "EliteExtratorExtractHere", "EliteExtratorExtractToFolder", "EliteExtratorListArchive"] {
        let legacy_key = format!("HKCU\\Software\\Classes\\SystemFileAssociations\\{ext}\\shell\\{legacy}");
        let _ = std::process::Command::new("reg").args(["delete", &legacy_key, "/f"]).output();
    }
    for (verb, label, flag) in verbs {
        for base in [
            format!("HKCU\\Software\\Classes\\SystemFileAssociations\\{ext}\\shell"),
            format!("HKCU\\Software\\Classes\\{ext}\\shell"),
        ] {
            let key = format!("{base}\\IlluminatiEliteExtractor{verb}");
            reg_add(&key, None, label);
            reg_add(&key, Some("MUIVerb"), label);
            reg_add(&key, Some("Icon"), exe);
            reg_add(&format!("{key}\\command"), None, &format!("\"{exe}\" {flag} \"%1\""));
        }
    }
}

#[cfg(windows)]
fn register_house_file_type(exe: &str, ext: &str) {
    let prog_id = "AEliteEstrangeira.IlluminatiArchive";
    reg_add(&format!("HKCU\\Software\\Classes\\{ext}"), None, prog_id);
    reg_add(&format!("HKCU\\Software\\Classes\\{prog_id}"), None, "Illuminati Archive");
    reg_add(&format!("HKCU\\Software\\Classes\\{prog_id}"), Some("FriendlyTypeName"), "Illuminati Archive");
    reg_add(&format!("HKCU\\Software\\Classes\\{ext}"), Some("Content Type"), "application/x-illuminati-archive");
    reg_add(&format!("HKCU\\Software\\Classes\\{ext}"), Some("PerceivedType"), "compressed");
    reg_add(&format!("HKCU\\Software\\Classes\\{ext}"), Some("AlwaysShowExt"), "");
    reg_add(&format!("HKCU\\Software\\Classes\\{prog_id}\\DefaultIcon"), None, &format!("{exe},0"));
    reg_add(&format!("HKCU\\Software\\Microsoft\\Windows\\CurrentVersion\\Explorer\\FileExts\\{ext}\\OpenWithProgids"), Some(prog_id), "");
    reg_add(&format!("HKCU\\Software\\Classes\\{prog_id}\\shell\\open\\command"), None, &format!("\"{exe}\" --list \"%1\""));
}

#[cfg(windows)]
fn register_context_menu_inner() {
    let exe = std::env::current_exe().map(|p| p.to_string_lossy().to_string()).unwrap_or_default();
    let exts = [".zip", ".7z", ".rar", ".tar", ".gz", ".bz2", ".xz", ".zst", ".tgz", ".illuminati", ".elite"];
    let verbs = [
        ("ExtractHere", "Extract here with A Elite Estrangeira", "--extract-here"),
        ("ExtractToFolder", "Extract to folder with A Elite Estrangeira", "--extract-to"),
        ("ListArchive", "List with A Elite Estrangeira", "--list"),
    ];
    for ext in exts {
        register_archive_extension(&exe, ext);
    }
    // .illuminati is the primary house format; legacy .elite stays readable.
    register_house_file_type(&exe, ".illuminati");
    register_house_file_type(&exe, ".elite");
    // Folder theme: Silver Graphite (no-text version), first theme.
    let _ = folder_theme_icon_path();
    for (base, arg) in [("HKCU\\Software\\Classes\\Directory\\shell", "%1"), ("HKCU\\Software\\Classes\\Directory\\Background\\shell", "%V")] {
        let key = format!("{base}\\IlluminatiEliteExtractorFolderThemeSilverGraphite");
        reg_add(&key, None, "Folder theme: Silver Graphite");
        reg_add(&key, Some("MUIVerb"), "Folder theme: Silver Graphite");
        reg_add(&key, Some("Icon"), &exe);
        reg_add(&format!("{key}\\command"), None, &format!("\"{exe}\" --folder-theme \"{arg}\""));
    }
    // Theme selection includes going back: restore the Windows default.
    for (base, arg) in [("HKCU\\Software\\Classes\\Directory\\shell", "%1"), ("HKCU\\Software\\Classes\\Directory\\Background\\shell", "%V")] {
        let key = format!("{base}\\IlluminatiEliteExtractorFolderThemeDefault");
        reg_add(&key, None, "Folder theme: Windows default");
        reg_add(&key, Some("MUIVerb"), "Folder theme: Windows default");
        reg_add(&key, Some("Icon"), &exe);
        reg_add(&format!("{key}\\command"), None, &format!("\"{exe}\" --folder-theme-default \"{arg}\""));
    }
    // Add to archive for any file and for folders.
    for base in ["HKCU\\Software\\Classes\\*\\shell", "HKCU\\Software\\Classes\\Directory\\shell"] {
        let key = format!("{base}\\IlluminatiEliteExtractorAddToArchive");
        reg_add(&key, None, "Add to archive with A Elite Estrangeira");
        reg_add(&key, Some("MUIVerb"), "Add to archive with A Elite Estrangeira");
        reg_add(&key, Some("Icon"), &exe);
        reg_add(&format!("{key}\\command"), None, &format!("\"{exe}\" --add \"%1\""));
    }
    let bg_add = "HKCU\\Software\\Classes\\Directory\\Background\\shell\\IlluminatiEliteExtractorAddToArchive";
    reg_add(bg_add, None, "Add to archive with A Elite Estrangeira");
    reg_add(bg_add, Some("MUIVerb"), "Add to archive with A Elite Estrangeira");
    reg_add(bg_add, Some("Icon"), &exe);
    reg_add(&format!("{bg_add}\\command"), None, &format!("\"{exe}\" --add \"%V\""));
    // Folders too, like 7-Zip: on a folder the verbs act on the archives
    // directly inside it. Directory\Background receives the folder as %V.
    for (verb, label, flag) in verbs {
        let key = format!("HKCU\\Software\\Classes\\Directory\\shell\\IlluminatiEliteExtractor{verb}");
        reg_add(&key, None, label);
        reg_add(&key, Some("MUIVerb"), label);
        reg_add(&key, Some("Icon"), &exe);
        reg_add(&format!("{key}\\command"), None, &format!("\"{exe}\" {flag} \"%1\""));
        let bg_key = format!("HKCU\\Software\\Classes\\Directory\\Background\\shell\\IlluminatiEliteExtractor{verb}");
        reg_add(&bg_key, None, label);
        reg_add(&bg_key, Some("MUIVerb"), label);
        reg_add(&bg_key, Some("Icon"), &exe);
        reg_add(&format!("{bg_key}\\command"), None, &format!("\"{exe}\" {flag} \"%V\""));
    }
}

#[tauri::command]
fn register_context_menu() -> Result<String, String> {
    #[cfg(windows)]
    {
        register_context_menu_inner();
        Ok("Context menu registered for archive files".to_string())
    }
    #[cfg(not(windows))]
    {
        Ok("Context menu registration is Windows-only".to_string())
    }
}

fn run_cli_if_requested() -> bool {
    let args: Vec<String> = std::env::args().collect();
    for pair in args.windows(2) {
        match pair[0].as_str() {
            "--folder-theme" => {
                let _ = apply_folder_theme(Path::new(&pair[1]));
                return true;
            }
            "--folder-theme-default" => {
                let _ = restore_default_folder_theme();
                return true;
            }
            "--extract-here" => {
                let p = Path::new(&pair[1]);
                if p.is_dir() {
                    if let Ok(read) = std::fs::read_dir(p) {
                        for entry in read.flatten() {
                            let file = entry.path();
                            if file.is_file() && list_archive_inner(&file).is_ok() {
                                let _ = extract_archive_inner(&file, p);
                            }
                        }
                    }
                } else if let Some(parent) = p.parent() {
                    let _ = extract_archive_inner(p, parent);
                }
                return true;
            }
            "--extract-to" => {
                let p = Path::new(&pair[1]);
                if p.is_dir() {
                    if let Ok(read) = std::fs::read_dir(p) {
                        for entry in read.flatten() {
                            let file = entry.path();
                            if file.is_file() && list_archive_inner(&file).is_ok() {
                                let stem = file.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| "extracted".to_string());
                                let stem = stem.trim_end_matches(".tar").to_string();
                                let _ = extract_archive_inner(&file, &p.join(stem));
                            }
                        }
                    }
                } else if let Some(parent) = p.parent() {
                    let stem = p.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| "extracted".to_string());
                    let stem = stem.trim_end_matches(".tar").to_string();
                    let _ = extract_archive_inner(p, &parent.join(stem));
                }
                return true;
            }
            "--list" => {
                let p = Path::new(&pair[1]);
                if p.is_dir() {
                    let mut text = String::new();
                    if let Ok(read) = std::fs::read_dir(p) {
                        for entry in read.flatten() {
                            let file = entry.path();
                            if file.is_file() {
                                if let Ok(entries) = list_archive_inner(&file) {
                                    text.push_str(&format!("{}\n", file.file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_default()));
                                    for e in entries {
                                        if e.is_dir {
                                            text.push_str(&format!("  [dir] {}\n", e.name));
                                        } else {
                                            text.push_str(&format!("  {} ({} bytes)\n", e.name, e.size));
                                        }
                                    }
                                }
                            }
                        }
                    }
                    let out = p.join("archives.contents.txt");
                    if std::fs::write(&out, text).is_ok() {
                        #[cfg(windows)]
                        {
                            let _ = std::process::Command::new("notepad.exe").arg(&out).spawn();
                        }
                    }
                    return true;
                }
                if let Ok(entries) = list_archive_inner(p) {
                    let stem = p.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| "archive".to_string());
                    let stem = stem.trim_end_matches(".tar").to_string();
                    if let Some(parent) = p.parent() {
                        let out = parent.join(format!("{stem}.contents.txt"));
                        let mut text = String::new();
                        for entry in entries {
                            if entry.is_dir {
                                text.push_str(&format!("[dir] {}\n", entry.name));
                            } else {
                                text.push_str(&format!("{} ({} bytes)\n", entry.name, entry.size));
                            }
                        }
                        if std::fs::write(&out, text).is_ok() {
                            #[cfg(windows)]
                            {
                                let _ = std::process::Command::new("notepad.exe").arg(&out).spawn();
                            }
                        }
                    }
                }
                return true;
            }
            _ => {}
        }
    }
    false
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
fn needs_window() -> bool {
    let args: Vec<String> = std::env::args().collect();
    for pair in args.windows(2) {
        if pair[0] == "--add" {
            return true;
        }
        if (pair[0] == "--extract-here" || pair[0] == "--extract-to") && illuminati_is_encrypted(Path::new(&pair[1])) {
            return true;
        }
    }
    false
}

pub fn run() {
    // Context-menu only: extraction never opens an application window. The
    // only windows are the Add to Archive settings dialog and the password
    // prompt for encrypted ELITE archives.
    if needs_window() {
        tauri::Builder::default()
            .plugin(tauri_plugin_dialog::init())
            .plugin(tauri_plugin_process::init())
            .invoke_handler(tauri::generate_handler![dialog_context, create_archive, extract_with_password, list_archive, extract_archive, extract_here, extract_to_folder, initial_path, register_context_menu])
            .setup(|_app| {
                #[cfg(windows)]
                register_context_menu_inner();
                Ok(())
            })
            .run(tauri::generate_context!())
            .expect("error while running the dialog");
        return;
    }
    if run_cli_if_requested() {
        return;
    }
    #[cfg(windows)]
    register_context_menu_inner();
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write as _;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("illuminati-extractor-test-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn zip_roundtrip_and_slip_block() {
        let dir = temp_dir("zip");
        let archive_path = dir.join("sample.zip");
        {
            let file = File::create(&archive_path).unwrap();
            let mut zip = zip::ZipWriter::new(file);
            let options = zip::write::SimpleFileOptions::default();
            zip.start_file("hello.txt", options).unwrap();
            zip.write_all(b"hello elite\n").unwrap();
            zip.start_file("../evil.txt", options).unwrap();
            zip.write_all(b"evil\n").unwrap();
            zip.finish().unwrap();
        }
        let listed = list_archive_inner(&archive_path).unwrap();
        assert_eq!(listed.len(), 2);
        let dest = dir.join("out");
        let result = extract_archive_inner(&archive_path, &dest);
        assert!(result.is_err());
        assert!(!dir.join("evil.txt").exists());
    }

    #[test]
    fn illuminati_roundtrip_and_encrypted() {
        let dir = temp_dir("illuminati");
        let src = dir.join("Template");
        std::fs::create_dir_all(&src).unwrap();
        std::fs::write(src.join("hello.txt"), b"hello elite\n").unwrap();
        let plain = dir.join("Template.illuminati");
        create_illuminati_archive(&src, &plain, 3, "").unwrap();
        let listed = list_archive_inner(&plain).unwrap();
        assert!(listed.iter().any(|e| e.name.ends_with("hello.txt")));
        let legacy = dir.join("Template.elite");
        std::fs::copy(&plain, &legacy).unwrap();
        assert!(list_archive_inner(&legacy).unwrap().iter().any(|e| e.name.ends_with("hello.txt")));
        let out = dir.join("out");
        let files = extract_archive_inner(&plain, &out).unwrap();
        assert_eq!(files.len(), 1);
        assert_eq!(std::fs::read_to_string(out.join("Template/hello.txt")).unwrap(), "hello elite\n");

        let encrypted = dir.join("Secret.illuminati");
        create_illuminati_archive(&src, &encrypted, 3, "correct horse").unwrap();
        assert!(illuminati_is_encrypted(&encrypted));
        assert!(extract_archive_inner(&encrypted, &dir.join("nope")).is_err());
        let out2 = dir.join("out2");
        let files2 = extract_archive_inner_with_password(&encrypted, &out2, Some("correct horse")).unwrap();
        assert_eq!(files2.len(), 1);
    }

    #[test]
    fn custom_extension_roundtrip_by_magic() {
        let dir = temp_dir("custom");
        let src = dir.join("Template");
        std::fs::create_dir_all(&src).unwrap();
        std::fs::write(src.join("hello.txt"), b"hello elite\n").unwrap();
        let created = create_archive(
            src.to_string_lossy().to_string(),
            dir.join("Template").to_string_lossy().to_string(),
            "custom".to_string(),
            3,
            String::new(),
            Some("vault".to_string()),
            false,
        ).unwrap();
        assert!(created.contains("Template.vault"));
        let renamed = dir.join("Template.bin");
        std::fs::rename(dir.join("Template.vault"), &renamed).unwrap();
        let out = dir.join("out");
        let files = extract_archive_inner(&renamed, &out).unwrap();
        assert_eq!(files.len(), 1);
        assert!(normalize_custom_extension("").is_err());
        assert!(normalize_custom_extension("../x").is_err());
        assert!(normalize_custom_extension("zip").is_err());
        assert_eq!(normalize_custom_extension(".Vault").unwrap(), "vault");
    }

    #[test]
    fn zip_create_roundtrip() {
        let dir = temp_dir("zipcreate");
        let src = dir.join("Template");
        std::fs::create_dir_all(&src).unwrap();
        std::fs::write(src.join("hello.txt"), b"hello elite\n").unwrap();
        let archive = dir.join("Template.zip");
        create_zip_archive(&src, &archive, 6).unwrap();
        let out = dir.join("out");
        let files = extract_archive_inner(&archive, &out).unwrap();
        assert_eq!(files.len(), 1);
    }

    #[test]
    fn folder_theme_icon_is_written() {
        let dir = temp_dir("theme");
        let folder = dir.join("Themed");
        std::fs::create_dir_all(&folder).unwrap();
        apply_folder_theme(&folder).unwrap();
        let icon = folder_theme_icon_path();
        assert_eq!(std::fs::metadata(&icon).unwrap().len(), FOLDER_THEME_ICON.len() as u64);
    }

    #[test]
    fn folder_theme_restore_runs() {
        restore_default_folder_theme().unwrap();
    }

    #[test]
    fn tar_gz_roundtrip() {
        let dir = temp_dir("targz");
        let archive_path = dir.join("sample.tar.gz");
        {
            let file = File::create(&archive_path).unwrap();
            let encoder = flate2::write::GzEncoder::new(file, flate2::Compression::default());
            let mut tar = tar::Builder::new(encoder);
            let data = b"hello elite\n";
            let mut header = tar::Header::new_gnu();
            header.set_size(data.len() as u64);
            header.set_cksum();
            tar.append_data(&mut header, "hello.txt", &data[..]).unwrap();
            tar.into_inner().unwrap().finish().unwrap();
        }
        let dest = dir.join("out");
        let files = extract_archive_inner(&archive_path, &dest).unwrap();
        assert_eq!(files.len(), 1);
        assert_eq!(std::fs::read_to_string(dest.join("hello.txt")).unwrap(), "hello elite\n");
        assert_eq!(files[0].sha256.len(), 64);
    }
}
