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

#[derive(Clone, Copy, PartialEq)]
enum Kind { Zip, SevenZ, Rar, Tar, TarGz, TarBz2, TarXz, TarZst, Gz, Bz2, Xz, Zst }

fn detect(path: &Path) -> Result<Kind, String> {
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
    }
    Ok(out)
}

fn extract_archive_inner(path: &Path, dest: &Path) -> Result<Vec<ExtractedInfo>, String> {
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

#[cfg(windows)]
fn register_context_menu_inner() {
    let exe = std::env::current_exe().map(|p| p.to_string_lossy().to_string()).unwrap_or_default();
    let exts = [".zip", ".7z", ".rar", ".tar", ".gz", ".bz2", ".xz", ".zst", ".tgz"];
    let verbs = [
        ("ExtractHere", "Extract here with A Elite Estrangeira", "--extract-here"),
        ("ExtractToFolder", "Extract to folder with A Elite Estrangeira", "--extract-to"),
        ("ListArchive", "List with A Elite Estrangeira", "--list"),
    ];
    for ext in exts {
        // Remove the legacy v1 verbs so Explorer shows one extractor only.
        for legacy in ["EliteExtratorHere", "EliteExtratorList", "EliteExtratorToFolder", "EliteExtratorExtractHere", "EliteExtratorExtractToFolder", "EliteExtratorListArchive"] {
            let legacy_key = format!("HKCU\\Software\\Classes\\SystemFileAssociations\\{ext}\\shell\\{legacy}");
            let _ = std::process::Command::new("reg").args(["delete", &legacy_key, "/f"]).output();
        }
        for (verb, label, flag) in verbs {
            let key = format!("HKCU\\Software\\Classes\\SystemFileAssociations\\{ext}\\shell\\IlluminatiEliteExtractor{verb}");
            reg_add(&key, None, label);
            reg_add(&key, Some("Icon"), &exe);
            reg_add(&format!("{key}\\command"), None, &format!("\"{exe}\" {flag} \"%1\""));
        }
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
            "--extract-here" => {
                let p = Path::new(&pair[1]);
                if let Some(parent) = p.parent() {
                    let _ = extract_archive_inner(p, parent);
                }
                return true;
            }
            "--extract-to" => {
                let p = Path::new(&pair[1]);
                if let Some(parent) = p.parent() {
                    let stem = p.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| "extracted".to_string());
                    let stem = stem.trim_end_matches(".tar").to_string();
                    let _ = extract_archive_inner(p, &parent.join(stem));
                }
                return true;
            }
            "--list" => {
                let p = Path::new(&pair[1]);
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
pub fn run() {
    // Context-menu only: this executable never opens an application window.
    // With no archive argument it only (re)registers the Explorer verbs.
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
