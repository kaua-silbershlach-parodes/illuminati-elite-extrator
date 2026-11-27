# Illuminati Elite Extractor

Archive extractor from **A Elite Estrangeira**, built to replace
third-party extractors in daily use. It lives in the Windows Explorer
context menu only — there is no application window to open.

## Supported formats

ZIP, JAR, EPUB, Office documents, 7Z, RAR, TAR, TAR.GZ, TAR.BZ2, TAR.XZ,
TAR.ZST, single-file GZ, BZ2, XZ and ZST, and the house **Illuminati**
format. Detection checks the Illuminati magic first, then extension
with a magic-byte fallback. Password-protected ZIP and 7Z archives are
not handled; encrypted Illuminati archives ask for their password.

## Illuminati format

The company archive format (`.illuminati`); legacy `.elite` archives
remain readable:

- TAR payload compressed with Zstandard (levels 1, 3, 9, 19)
- SHA-256 integrity verified on extraction
- Optional AES-256-GCM password encryption with Argon2id key derivation
- `.illuminati` files carry the house icon and the "Illuminati Archive" file type
- Custom extension creation writes the same Illuminati container under
  a user-chosen extension; magic-first detection reopens it

## Windows context menu

On archive files, on folders, and on folder background:

- **Extract here with A Elite Estrangeira**
- **Extract to folder with A Elite Estrangeira**
- **List with A Elite Estrangeira** — writes a contents text file next
  to the archive and opens it in Notepad
- **Add to archive with A Elite Estrangeira** — archive name, format
  (Illuminati, custom extension, or ZIP), compression level, and optional password

On folders:

- **Folder theme: Silver Graphite** — applies the Silver Graphite
  folder theme system-wide and refreshes Explorer automatically
- **Restore Windows default folder theme**

Running the executable with no arguments only registers the context
menu again.

## Safety

Archive entries are checked before extraction. Entries that would
escape the destination folder are blocked. Extracted files are
reported with size and SHA-256.

## Releases and updates

Installers are published in this repository's GitHub Releases. The
extractor checks the release feed for signed updates.

Copyright (c) A Elite Estrangeira.
