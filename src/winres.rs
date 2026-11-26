//! Zero-dependency Windows resource (.res / COFF) generator.
//!
//! `rc.exe` / `windres` / `llvm-rc` are frequently unavailable in cross toolchains
//! (and zig ships no `rc` subcommand), which silently drops the application icon
//! and the VERSIONINFO block from every Windows binary. This module builds the
//! exact same COFF `.res` object by hand from `resources/icon.ico`,
//! `resources/app.manifest` and a version table, so the icon and version
//! resource are embedded on *every* Windows build with no external tools.

use std::fs;
use std::path::Path;

const RT_ICON: u16 = 3;
const RT_GROUP_ICON: u16 = 14;
const RT_VERSION: u16 = 16;
const RT_MANIFEST: u16 = 24;

const LANG_EN_US: u16 = 0x0409;
const CODEPAGE_UTF16: u32 = 1200;

#[derive(Clone, Debug)]
pub struct IconImage {
    pub width: u8,
    pub height: u8,
    pub color_count: u8,
    pub planes: u16,
    pub bit_count: u16,
    pub data: Vec<u8>,
}

#[derive(Clone, Debug)]
pub struct VersionInfo {
    /// (major, minor, build, revision) packed into FILEVERSION / PRODUCTVERSION.
    pub file_version: (u16, u16, u16, u16),
    pub product_version: (u16, u16, u16, u16),
    pub company_name: String,
    pub file_description: String,
    pub file_version_str: String,
    pub internal_name: String,
    pub legal_copyright: String,
    pub original_filename: String,
    pub product_name: String,
    pub product_version_str: String,
}

impl VersionInfo {
    /// Build a `VersionInfo` from a `major.minor.patch` crate version.
    pub fn from_crate_version(version: &str, internal_name: &str, description: &str) -> Self {
        let clean = version.split(['-', '+']).next().unwrap_or("0.0.0");
        let nums: Vec<u16> = clean
            .split('.')
            .map(|p| p.trim().parse::<u16>().unwrap_or(0))
            .collect();
        let major = nums.first().copied().unwrap_or(0);
        let minor = nums.get(1).copied().unwrap_or(0);
        let patch = nums.get(2).copied().unwrap_or(0);
        let revision = nums.get(3).copied().unwrap_or(0);
        let dotted = format!("{}.{}.{}.{}", major, minor, patch, revision);
        Self {
            file_version: (major, minor, patch, revision),
            product_version: (major, minor, patch, revision),
            company_name: "Aircode Desktop".to_string(),
            file_description: description.to_string(),
            file_version_str: dotted.clone(),
            internal_name: internal_name.to_string(),
            legal_copyright: "Copyright (C) 2026 Aircode Contributors".to_string(),
            original_filename: format!("{}.exe", internal_name),
            product_name: "AirCode".to_string(),
            product_version_str: format!("{}.{}.{}", major, minor, patch),
        }
    }
}

fn read_u16(buf: &[u8], off: usize) -> Result<u16, String> {
    if off + 2 > buf.len() {
        return Err("ico: truncated u16".to_string());
    }
    Ok(u16::from_le_bytes([buf[off], buf[off + 1]]))
}

fn read_u32(buf: &[u8], off: usize) -> Result<u32, String> {
    if off + 4 > buf.len() {
        return Err("ico: truncated u32".to_string());
    }
    Ok(u32::from_le_bytes([
        buf[off],
        buf[off + 1],
        buf[off + 2],
        buf[off + 3],
    ]))
}

/// Parse a `.ico` container into the individual image payloads that the
/// resource tree stores as `RT_ICON` entries.
pub fn parse_ico(bytes: &[u8]) -> Result<Vec<IconImage>, String> {
    if bytes.len() < 6 {
        return Err("ico: file too small".to_string());
    }
    let reserved = read_u16(bytes, 0)?;
    let kind = read_u16(bytes, 2)?;
    let count = read_u16(bytes, 4)? as usize;
    if reserved != 0 || kind != 1 {
        return Err("ico: not an icon container".to_string());
    }
    if count == 0 {
        return Err("ico: no images".to_string());
    }
    let mut out: Vec<IconImage> = Vec::with_capacity(count);
    for i in 0..count {
        let e = 6 + i * 16;
        if e + 16 > bytes.len() {
            return Err("ico: truncated directory".to_string());
        }
        let width = bytes[e];
        let height = bytes[e + 1];
        let color_count = bytes[e + 2];
        let planes = read_u16(bytes, e + 4)?;
        let bit_count = read_u16(bytes, e + 6)?;
        let size = read_u32(bytes, e + 8)? as usize;
        let offset = read_u32(bytes, e + 12)? as usize;
        if offset + size > bytes.len() {
            return Err(format!("ico: image {} out of bounds", i));
        }
        out.push(IconImage {
            width,
            height,
            color_count,
            planes,
            bit_count,
            data: bytes[offset..offset + size].to_vec(),
        });
    }
    Ok(out)
}

/// Build the `RT_GROUP_ICON` payload (a `GRPICONDIR`) for parsed images.
pub fn make_group_icon(images: &[IconImage]) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(&0u16.to_le_bytes()); // reserved
    out.extend_from_slice(&1u16.to_le_bytes()); // type = icon
    out.extend_from_slice(&(images.len() as u16).to_le_bytes());
    for (i, img) in images.iter().enumerate() {
        out.push(img.width);
        out.push(img.height);
        out.push(img.color_count);
        out.push(0); // reserved
        out.extend_from_slice(&img.planes.to_le_bytes());
        out.extend_from_slice(&img.bit_count.to_le_bytes());
        out.extend_from_slice(&(img.data.len() as u32).to_le_bytes());
        out.extend_from_slice(&((i + 1) as u16).to_le_bytes()); // RT_ICON id
    }
    out
}

fn push_utf16_padded(out: &mut Vec<u8>, text: &str) {
    for unit in text.encode_utf16() {
        out.extend_from_slice(&unit.to_le_bytes());
    }
    out.extend_from_slice(&0u16.to_le_bytes()); // NUL terminator
    while out.len() % 4 != 0 {
        out.extend_from_slice(&0u16.to_le_bytes());
    }
}

/// Assemble a `VS_VERSION_INFO` block (the `RT_VERSION` payload).
pub fn make_version_info(v: &VersionInfo) -> Vec<u8> {
    let mut root: Vec<u8> = Vec::new();
    root.extend_from_slice(&0u16.to_le_bytes()); // wLength (patched later)
    root.extend_from_slice(&0u16.to_le_bytes()); // wValueLength
    root.extend_from_slice(&0u16.to_le_bytes()); // wType = binary
    push_utf16_padded(&mut root, "VS_VERSION_INFO");

    // VS_FIXEDFILEINFO (13 DWORDs = 52 bytes)
    let (fmaj, fmin, fbld, frev) = v.file_version;
    let (pmaj, pmin, pbld, prev) = v.product_version;
    root.extend_from_slice(&0xFEEF_04BDu32.to_le_bytes()); // dwSignature
    root.extend_from_slice(&0x0001_0000u32.to_le_bytes()); // dwStrucVersion = 1.0
    root.extend_from_slice(&((fmaj as u32) << 16 | fmin as u32).to_le_bytes());
    root.extend_from_slice(&((fbld as u32) << 16 | frev as u32).to_le_bytes());
    root.extend_from_slice(&((pmaj as u32) << 16 | pmin as u32).to_le_bytes());
    root.extend_from_slice(&((pbld as u32) << 16 | prev as u32).to_le_bytes());
    root.extend_from_slice(&0x3Fu32.to_le_bytes()); // dwFileFlagsMask
    root.extend_from_slice(&0u32.to_le_bytes()); // dwFileFlags
    root.extend_from_slice(&0x0004_0004u32.to_le_bytes()); // dwFileOS = NT
    root.extend_from_slice(&0x0000_0001u32.to_le_bytes()); // dwFileType = application
    root.extend_from_slice(&0u32.to_le_bytes()); // dwFileSubtype
    root.extend_from_slice(&0u32.to_le_bytes()); // dwFileDateMS
    root.extend_from_slice(&0u32.to_le_bytes()); // dwFileDateLS

    // ---- StringFileInfo > StringTable > String[] ----
    let strings: Vec<(&str, &str)> = vec![
        ("CompanyName", v.company_name.as_str()),
        ("FileDescription", v.file_description.as_str()),
        ("FileVersion", v.file_version_str.as_str()),
        ("InternalName", v.internal_name.as_str()),
        ("LegalCopyright", v.legal_copyright.as_str()),
        ("OriginalFilename", v.original_filename.as_str()),
        ("ProductName", v.product_name.as_str()),
        ("ProductVersion", v.product_version_str.as_str()),
    ];
    let mut string_blocks: Vec<Vec<u8>> = Vec::new();
    for (key, value) in &strings {
        let mut b: Vec<u8> = Vec::new();
        b.extend_from_slice(&0u16.to_le_bytes()); // wLength (patched)
        b.extend_from_slice(&(value.encode_utf16().count() as u16).to_le_bytes());
        b.extend_from_slice(&1u16.to_le_bytes()); // wType = text
        push_utf16_padded(&mut b, key);
        push_utf16_padded(&mut b, value);
        patch_len(&mut b);
        string_blocks.push(b);
    }
    let mut table: Vec<u8> = Vec::new();
    table.extend_from_slice(&0u16.to_le_bytes()); // wLength (patched)
    table.extend_from_slice(&0u16.to_le_bytes()); // wValueLength
    table.extend_from_slice(&1u16.to_le_bytes()); // wType = text
    push_utf16_padded(&mut table, "040904B0");
    for b in &string_blocks {
        while table.len() % 4 != 0 {
            table.extend_from_slice(&[0, 0]);
        }
        table.extend_from_slice(b);
    }
    patch_len(&mut table);

    let mut sfi: Vec<u8> = Vec::new();
    sfi.extend_from_slice(&0u16.to_le_bytes()); // wLength (patched)
    sfi.extend_from_slice(&0u16.to_le_bytes()); // wValueLength
    sfi.extend_from_slice(&1u16.to_le_bytes()); // wType = text
    push_utf16_padded(&mut sfi, "StringFileInfo");
    while sfi.len() % 4 != 0 {
        sfi.extend_from_slice(&[0, 0]);
    }
    sfi.extend_from_slice(&table);
    patch_len(&mut sfi);

    // ---- VarFileInfo > Var{Translation} ----
    let mut var_block: Vec<u8> = Vec::new();
    var_block.extend_from_slice(&0u16.to_le_bytes()); // wLength (patched)
    var_block.extend_from_slice(&4u16.to_le_bytes()); // wValueLength = sizeof(DWORD)
    var_block.extend_from_slice(&0u16.to_le_bytes()); // wType = binary
    push_utf16_padded(&mut var_block, "Translation");
    // The Var value is a single DWORD: low word = language, high word = codepage.
    let translation: u32 = (LANG_EN_US as u32) | ((CODEPAGE_UTF16 as u32) << 16);
    var_block.extend_from_slice(&translation.to_le_bytes());
    patch_len(&mut var_block);

    let mut vfi: Vec<u8> = Vec::new();
    vfi.extend_from_slice(&0u16.to_le_bytes()); // wLength (patched)
    vfi.extend_from_slice(&0u16.to_le_bytes()); // wValueLength
    vfi.extend_from_slice(&0u16.to_le_bytes()); // wType = binary
    push_utf16_padded(&mut vfi, "VarFileInfo");
    while vfi.len() % 4 != 0 {
        vfi.extend_from_slice(&[0, 0]);
    }
    vfi.extend_from_slice(&var_block);
    patch_len(&mut vfi);

    root.extend_from_slice(&sfi);
    while root.len() % 4 != 0 {
        root.extend_from_slice(&[0, 0]);
    }
    root.extend_from_slice(&vfi);
    patch_len(&mut root);
    root
}

fn patch_len(buf: &mut [u8]) {
    let len = buf.len() as u16;
    buf[0] = (len & 0xFF) as u8;
    buf[1] = ((len >> 8) & 0xFF) as u8;
}

// ---------------------------------------------------------------------------
// COFF resource object
// ---------------------------------------------------------------------------

/// Growable resource blob with deferred (position, target) patch records.
struct Blob {
    data: Vec<u8>,
    patches: Vec<(usize, usize)>,
}

impl Blob {
    fn new() -> Self {
        Self {
            data: Vec::new(),
            patches: Vec::new(),
        }
    }
    fn len(&self) -> usize {
        self.data.len()
    }
    fn align4(&mut self) {
        while self.data.len() % 4 != 0 {
            self.data.push(0);
        }
    }
    fn u16(&mut self, v: u16) {
        self.data.extend_from_slice(&v.to_le_bytes());
    }
    fn u32(&mut self, v: u32) {
        self.data.extend_from_slice(&v.to_le_bytes());
    }
    fn raw(&mut self, b: &[u8]) {
        self.data.extend_from_slice(b);
    }
    /// Reserve 4 bytes and remember to patch it with `target` later.
    fn reserve_u32(&mut self, target: usize) {
        let slot = self.data.len();
        self.u32(0);
        self.patches.push((slot, target));
    }
    fn apply_patches(&mut self) {
        for (slot, target) in &self.patches {
            let v = *target as u32;
            self.data[*slot] = (v & 0xFF) as u8;
            self.data[*slot + 1] = ((v >> 8) & 0xFF) as u8;
            self.data[*slot + 2] = ((v >> 16) & 0xFF) as u8;
            self.data[*slot + 3] = ((v >> 24) & 0xFF) as u8;
        }
    }
    /// `IMAGE_RESOURCE_DIRECTORY` header with `count` id entries.
    fn dir_header(&mut self, count: usize) -> usize {
        self.u32(0); // Characteristics
        self.u32(0); // TimeDateStamp
        self.u16(0); // MajorVersion
        self.u16(0); // MinorVersion
        self.u16(0); // NumberOfNamedEntries
        self.u16(count as u16); // NumberOfIdEntries
        self.len()
    }
}

/// Serialize a resource tree into the `.rsrc` blob carried by a `.res` object.
///
/// Layout: level-1 directory (types) -> level-2 (names) -> level-3 (languages)
/// -> `IMAGE_RESOURCE_DATA_ENTRY[]` -> payloads. Offsets are relative to the
/// start of the blob, which is what the PE loader expects.
fn serialize_resources(entries: &[(u16, u16, u16, Vec<u8>)]) -> Vec<u8> {
    let mut types: Vec<u16> = entries.iter().map(|e| e.0).collect();
    types.sort_unstable();
    types.dedup();

    let mut b = Blob::new();
    // (slot position of the OffsetToData field, directory offset to patch in)
    let mut dir_patches: Vec<(usize, usize)> = Vec::new();
    // (slot position, index into `entries`)

    // --- level 1: types ---
    b.align4();
    b.dir_header(types.len());
    let mut l1_slots: Vec<(usize, u16)> = Vec::new();
    for t in &types {
        let slot = b.len();
        b.u32(*t as u32);
        b.reserve_u32(usize::MAX); // patched once level-2 offsets are known
        l1_slots.push((slot, *t));
    }

    // --- level 2: names per type ---
    let mut l2_slots: Vec<(usize, u16, u16)> = Vec::new();
    for (slot, t) in &l1_slots {
        let mut names: Vec<u16> = entries
            .iter()
            .filter(|e| e.0 == *t)
            .map(|e| e.1)
            .collect();
        names.sort_unstable();
        names.dedup();

        b.align4();
        let dir_off = b.len();
        b.dir_header(names.len());
        dir_patches.push((*slot, dir_off));
        for n in names {
            let nslot = b.len();
            b.u32(n as u32);
            b.reserve_u32(usize::MAX);
            l2_slots.push((nslot, *t, n));
        }
    }

    // --- level 3: languages per (type, name) ---
    let mut l3_slots: Vec<(usize, usize)> = Vec::new();
    for (slot, t, n) in &l2_slots {
        let mut langs: Vec<usize> = entries
            .iter()
            .enumerate()
            .filter(|(_, e)| e.0 == *t && e.1 == *n)
            .map(|(i, _)| i)
            .collect();
        langs.sort_by_key(|i| entries[*i].2);

        b.align4();
        let dir_off = b.len();
        b.dir_header(langs.len());
        dir_patches.push((*slot, dir_off));
        for i in langs {
            b.u32(entries[i].2 as u32);
            b.reserve_u32(usize::MAX);
            l3_slots.push((b.len() - 4, i));
        }
    }

    // --- data entries ---
    b.align4();
    let mut data_entry_offsets: Vec<usize> = Vec::with_capacity(entries.len());
    for e in entries {
        b.align4();
        let de = b.len();
        b.reserve_u32(usize::MAX); // OffsetToData
        b.u32(e.3.len() as u32); // Size
        b.u32(0); // CodePage
        b.u32(0); // Reserved
        data_entry_offsets.push(de);
    }

    // --- payloads ---
    let mut payload_offsets: Vec<usize> = Vec::with_capacity(entries.len());
    for e in entries {
        b.align4();
        payload_offsets.push(b.len());
        b.raw(&e.3);
    }

    // --- resolve patches ---
    for (slot, dir_off) in &dir_patches {
        b.patches.push((*slot + 4, dir_off | 0x8000_0000));
    }
    for (slot, i) in &l3_slots {
        b.patches.push((*slot, data_entry_offsets[*i]));
    }
    for (i, de) in data_entry_offsets.iter().enumerate() {
        b.patches.push((*de, payload_offsets[i]));
    }
    b.apply_patches();
    // Keep the section payload DWORD aligned (harmless for the PE loader).
    while b.data.len() % 4 != 0 {
        b.data.push(0);
    }
    b.data
}

/// Wrap a `.rsrc` blob in a minimal single-section COFF object (`.res`).
fn wrap_coff(machine: u16, rsrc: &[u8]) -> Vec<u8> {
    let mut out: Vec<u8> = Vec::new();
    // IMAGE_FILE_HEADER
    out.extend_from_slice(&machine.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes()); // NumberOfSections
    out.extend_from_slice(&0u32.to_le_bytes()); // TimeDateStamp
    out.extend_from_slice(&0u32.to_le_bytes()); // PointerToSymbolTable
    out.extend_from_slice(&0u32.to_le_bytes()); // NumberOfSymbols
    out.extend_from_slice(&0u16.to_le_bytes()); // SizeOfOptionalHeader
    out.extend_from_slice(&0u16.to_le_bytes()); // Characteristics

    // IMAGE_SECTION_HEADER for ".rsrc"
    let mut name = [0u8; 8];
    let n = b".rsrc";
    name[..n.len()].copy_from_slice(n);
    out.extend_from_slice(&name);
    out.extend_from_slice(&(rsrc.len() as u32).to_le_bytes()); // VirtualSize
    out.extend_from_slice(&0u32.to_le_bytes()); // VirtualAddress
    out.extend_from_slice(&(rsrc.len() as u32).to_le_bytes()); // SizeOfRawData
    out.extend_from_slice(&60u32.to_le_bytes()); // PointerToRawData
    out.extend_from_slice(&0u32.to_le_bytes()); // PointerToRelocations
    out.extend_from_slice(&0u32.to_le_bytes()); // PointerToLinenumbers
    out.extend_from_slice(&0u16.to_le_bytes()); // NumberOfRelocations
    out.extend_from_slice(&0u16.to_le_bytes()); // NumberOfLinenumbers
    out.extend_from_slice(&0x4000_0040u32.to_le_bytes()); // INITIALIZED_DATA | READ

    out.extend_from_slice(rsrc);
    out
}

/// High-level entry point: build the full `.res` bytes for the application.
pub fn generate(
    machine: u16,
    ico: &[u8],
    manifest: Option<&[u8]>,
    version: &VersionInfo,
) -> Result<Vec<u8>, String> {
    let images = parse_ico(ico)?;
    let group = make_group_icon(&images);

    let mut entries: Vec<(u16, u16, u16, Vec<u8>)> = Vec::new();
    for (i, img) in images.iter().enumerate() {
        entries.push((RT_ICON, (i + 1) as u16, LANG_EN_US, img.data.clone()));
    }
    entries.push((RT_GROUP_ICON, 1, LANG_EN_US, group));
    entries.push((RT_VERSION, 1, LANG_EN_US, make_version_info(version)));
    if let Some(m) = manifest {
        entries.push((RT_MANIFEST, 1, LANG_EN_US, m.to_vec()));
    }

    let rsrc = serialize_resources(&entries);
    Ok(wrap_coff(machine, &rsrc))
}

/// Convenience wrapper that reads the assets straight off disk.
pub fn generate_from_paths(
    machine: u16,
    ico_path: &Path,
    manifest_path: Option<&Path>,
    version: &VersionInfo,
) -> Result<Vec<u8>, String> {
    let ico = fs::read(ico_path).map_err(|e| format!("reading {}: {}", ico_path.display(), e))?;
    let manifest = match manifest_path {
        Some(p) => Some(
            fs::read(p).map_err(|e| format!("reading {}: {}", p.display(), e))?,
        ),
        None => None,
    };
    generate(machine, &ico, manifest.as_deref(), version)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A tiny but structurally valid 2-image ICO (16x16 + 32x32, 32bpp BMP payloads).
    fn sample_ico() -> Vec<u8> {
        let mut img16 = vec![0u8; 40];
        let mut img32 = vec![0u8; 88];
        img16[..6].copy_from_slice(&[1, 2, 3, 4, 5, 6]);
        img32[..6].copy_from_slice(&[9, 8, 7, 6, 5, 4]);
        let mut ico = Vec::new();
        ico.extend_from_slice(&0u16.to_le_bytes());
        ico.extend_from_slice(&1u16.to_le_bytes());
        ico.extend_from_slice(&2u16.to_le_bytes());
        // entry 1
        ico.extend_from_slice(&[16, 16, 0, 0]);
        ico.extend_from_slice(&1u16.to_le_bytes());
        ico.extend_from_slice(&32u16.to_le_bytes());
        ico.extend_from_slice(&(img16.len() as u32).to_le_bytes());
        ico.extend_from_slice(&((6 + 2 * 16) as u32).to_le_bytes());
        // entry 2
        ico.extend_from_slice(&[32, 32, 0, 0]);
        ico.extend_from_slice(&1u16.to_le_bytes());
        ico.extend_from_slice(&32u16.to_le_bytes());
        ico.extend_from_slice(&(img32.len() as u32).to_le_bytes());
        let off2 = 6 + 2 * 16 + img16.len();
        ico.extend_from_slice(&(off2 as u32).to_le_bytes());
        ico.extend_from_slice(&img16);
        ico.extend_from_slice(&img32);
        ico
    }

    #[test]
    fn test_parse_ico_extracts_every_image() {
        let images = parse_ico(&sample_ico()).expect("valid ico");
        assert_eq!(images.len(), 2);
        assert_eq!(images[0].width, 16);
        assert_eq!(images[0].height, 16);
        assert_eq!(images[0].data.len(), 40);
        assert_eq!(images[1].width, 32);
        assert_eq!(images[1].height, 32);
        assert_eq!(images[1].data.len(), 88);
        assert_eq!(images[1].data[..6], [9, 8, 7, 6, 5, 4]);
    }

    #[test]
    fn test_parse_ico_rejects_garbage() {
        assert!(parse_ico(&[0, 1]).is_err());
        let mut bad = sample_ico();
        bad[2] = 2; // type != 1
        assert!(parse_ico(&bad).is_err());
    }

    #[test]
    fn test_group_icon_shape() {
        let images = parse_ico(&sample_ico()).unwrap();
        let group = make_group_icon(&images);
        assert_eq!(read_u16(&group, 0).unwrap(), 0); // reserved
        assert_eq!(read_u16(&group, 2).unwrap(), 1); // type
        assert_eq!(read_u16(&group, 4).unwrap(), 2); // count
                                                      // first entry: w,h,colors,reserved,planes,bits,bytes,id
        assert_eq!(group[6], 16);
        assert_eq!(group[7], 16);
        assert_eq!(read_u16(&group, 10).unwrap(), 1); // planes
        assert_eq!(read_u16(&group, 12).unwrap(), 32); // bit count
        assert_eq!(read_u32(&group, 14).unwrap(), 40); // bytes in res
        assert_eq!(read_u16(&group, 18).unwrap(), 1); // RT_ICON id
        // second entry starts at 6 + 14; its id field is at +12
        assert_eq!(read_u16(&group, 6 + 14 + 12).unwrap(), 2); // second id
    }

    #[test]
    fn test_version_info_lengths_are_self_consistent() {
        let v = VersionInfo::from_crate_version(
            "2.2.0",
            "aircode",
            "AirCode — Pure Rust",
        );
        let block = make_version_info(&v);
        // Root length prefix covers the entire block.
        assert_eq!(read_u16(&block, 0).unwrap() as usize, block.len());
        assert_eq!(read_u16(&block, 2).unwrap(), 0); // wValueLength (binary)
        assert_eq!(read_u16(&block, 4).unwrap(), 0); // wType = binary
        let key: Vec<u16> = block[6..6 + 32]
            .chunks_exact(2)
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .collect();
        let key: String = char::decode_utf16(key)
            .map(|r| r.unwrap_or('\u{FFFD}'))
            .collect();
        assert!(key.starts_with("VS_VERSION_INFO"));
        // VS_FIXEDFILEINFO sits after the padded key (40 bytes of header+key).
        assert_eq!(read_u32(&block, 40).unwrap(), 0xFEEF_04BD); // dwSignature
        assert_eq!(read_u32(&block, 44).unwrap(), 0x0001_0000); // dwStrucVersion
        // FileVersion 2.2.0.0 => MS 0x0002_0002 / LS 0x0000_0000
        assert_eq!(read_u32(&block, 48).unwrap(), 0x0002_0002);
        assert_eq!(read_u32(&block, 52).unwrap(), 0x0000_0000);
        assert_eq!(read_u32(&block, 56).unwrap(), 0x0002_0002); // ProductVersionMS
        assert_eq!(read_u32(&block, 88).unwrap(), 0); // dwFileDateLS (last dword)
        assert!(block.len() % 4 == 0);
    }

    #[test]
    fn test_coff_wrapper_shape() {
        let images = parse_ico(&sample_ico()).unwrap();
        let mut entries: Vec<(u16, u16, u16, Vec<u8>)> = Vec::new();
        for (i, img) in images.iter().enumerate() {
            entries.push((RT_ICON, (i + 1) as u16, LANG_EN_US, img.data.clone()));
        }
        entries.push((RT_GROUP_ICON, 1, LANG_EN_US, make_group_icon(&images)));
        let rsrc = serialize_resources(&entries);
        assert!(rsrc.len() % 4 == 0);

        let coff = wrap_coff(0x8664, &rsrc);
        assert_eq!(read_u16(&coff, 0).unwrap(), 0x8664); // machine
        assert_eq!(read_u16(&coff, 2).unwrap(), 1); // one section
        assert_eq!(read_u16(&coff, 16).unwrap(), 0); // no optional header
        assert_eq!(&coff[20..25], b".rsrc");
        assert_eq!(read_u32(&coff, 20 + 16).unwrap() as usize, rsrc.len()); // raw size
        assert_eq!(read_u32(&coff, 20 + 20).unwrap(), 60); // raw ptr
        assert_eq!(coff.len(), 60 + rsrc.len());
    }

    #[test]
    fn test_resource_tree_walks_cleanly() {
        let images = parse_ico(&sample_ico()).unwrap();
        let mut entries: Vec<(u16, u16, u16, Vec<u8>)> = Vec::new();
        for (i, img) in images.iter().enumerate() {
            entries.push((RT_ICON, (i + 1) as u16, LANG_EN_US, img.data.clone()));
        }
        entries.push((RT_GROUP_ICON, 1, LANG_EN_US, make_group_icon(&images)));
        entries.push((RT_MANIFEST, 1, LANG_EN_US, b"<assembly/>".to_vec()));

        let rsrc = serialize_resources(&entries);
        let read_u32_at = |o: usize| read_u32(&rsrc, o).unwrap();

        // Level 1 directory: 16-byte header + 3 id entries (8 bytes each)
        assert_eq!(read_u16(&rsrc, 12).unwrap(), 0); // NumberOfNamedEntries
        assert_eq!(read_u16(&rsrc, 14).unwrap(), 3); // NumberOfIdEntries
        let mut level2 = Vec::new();
        for i in 0..3 {
            let e = 16 + i * 8;
            let name = read_u32_at(e);
            let off = read_u32_at(e + 4);
            assert!(name == RT_ICON as u32 || name == RT_GROUP_ICON as u32 || name == RT_MANIFEST as u32);
            assert_eq!(off & 0x8000_0000, 0x8000_0000, "child must be a directory");
            level2.push((name, (off & 0x7FFF_FFFF) as usize));
        }
        assert_eq!(level2.len(), 3);

        for (name, dir_off) in level2 {
            let named = read_u16(&rsrc, dir_off + 12).unwrap() as usize;
            let ids = read_u16(&rsrc, dir_off + 14).unwrap() as usize;
            assert_eq!(named, 0);
            assert!(ids >= 1);
            let expected = if name == RT_ICON as u32 {
                images.len()
            } else {
                1
            };
            assert_eq!(ids, expected);
            for i in 0..ids {
                let e = dir_off + 16 + i * 8;
                let off = read_u32_at(e + 4);
                assert_eq!(off & 0x8000_0000, 0x8000_0000, "level-3 must be a dir");
                let l3 = (off & 0x7FFF_FFFF) as usize;
                assert_eq!(read_u16(&rsrc, l3 + 14).unwrap(), 1, "one language");
                let doff = read_u32_at(l3 + 16 + 4);
                assert_eq!(doff & 0x8000_0000, 0, "leaf must be a data entry");
                let de = (doff & 0x7FFF_FFFF) as usize;
                let size = read_u32_at(de + 4);
                let data_off = read_u32_at(de);
                assert!(data_off as usize + size as usize <= rsrc.len());
            }
        }
    }

    #[test]
    fn test_generate_end_to_end() {
        let ico = sample_ico();
        let v = VersionInfo::from_crate_version("2.2.0", "aircode", "test");
        let res = generate(0x8664, &ico, Some(b"<assembly/>"), &v).expect("generate res");
        assert!(res.len() > 60);
        assert_eq!(&res[20..25], b".rsrc");
    }
}
