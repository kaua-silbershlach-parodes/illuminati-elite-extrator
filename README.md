# Illuminati Elite Extractor

Archive extractor from **A Elite Estrangeira**, built to replace third-party extractors in daily use.

## Supported formats

ZIP, 7Z, RAR, TAR, TAR.GZ, TAR.BZ2, TAR.XZ, TAR.ZST, GZ, BZ2, XZ and ZST.

## Safety

Archive entries are checked before extraction. Entries that would escape the destination folder are blocked. Extracted files are reported with size and SHA-256.

## Windows context menu

This is a context-menu extractor only. It does not open an application window. On Windows it registers Explorer context-menu entries for archive files:

- Extract here with A Elite Estrangeira
- Extract to folder with A Elite Estrangeira
- List with A Elite Estrangeira (writes a contents text file next to the archive and opens it in Notepad)

Running the executable with no archive only registers the context menu again.

## Releases

Installers are published in this repository's GitHub Releases.
