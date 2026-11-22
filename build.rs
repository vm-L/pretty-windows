use std::env;
use std::fs;
use std::path::{Path, PathBuf};

const RT_ICON: u16 = 3;
const RT_GROUP_ICON: u16 = 14;

fn read_u16_le(bytes: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes([bytes[offset], bytes[offset + 1]])
}

fn read_u32_le(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes([
        bytes[offset],
        bytes[offset + 1],
        bytes[offset + 2],
        bytes[offset + 3],
    ])
}

fn push_res_record(
    out: &mut Vec<u8>,
    res_type: u16,
    res_id: u16,
    memory_flags: u16,
    data: &[u8],
) {
    // Standard 32-byte Win32 .res resource header (numeric Type and Name ordinals).
    out.extend_from_slice(&(data.len() as u32).to_le_bytes()); // DataSize
    out.extend_from_slice(&32u32.to_le_bytes()); // HeaderSize
    out.extend_from_slice(&0xFFFFu16.to_le_bytes()); // Type ordinal marker
    out.extend_from_slice(&res_type.to_le_bytes());
    out.extend_from_slice(&0xFFFFu16.to_le_bytes()); // Name ordinal marker
    out.extend_from_slice(&res_id.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes()); // DataVersion
    out.extend_from_slice(&memory_flags.to_le_bytes()); // MemoryFlags
    out.extend_from_slice(&0u16.to_le_bytes()); // LanguageId (neutral)
    out.extend_from_slice(&0u32.to_le_bytes()); // Version
    out.extend_from_slice(&0u32.to_le_bytes()); // Characteristics

    out.extend_from_slice(data);
    while !out.len().is_multiple_of(4) {
        out.push(0);
    }
}

fn compile_ico_to_res(ico_path: &Path, res_path: &Path) {
    let ico = fs::read(ico_path).expect("failed to read assets/PrettyWindows.ico");
    assert!(ico.len() >= 6, "invalid ICO header");
    let reserved = read_u16_le(&ico, 0);
    let icon_type = read_u16_le(&ico, 2);
    let count = read_u16_le(&ico, 4) as usize;
    assert_eq!(reserved, 0, "ICO idReserved must be 0");
    assert_eq!(icon_type, 1, "ICO idType must be 1");
    assert!(count > 0, "ICO must contain at least one icon image");
    assert!(ico.len() >= 6 + count * 16, "truncated ICO directory");

    let mut res = Vec::with_capacity(ico.len() + 256);

    // 32-byte empty RES signature header.
    res.extend_from_slice(&0u32.to_le_bytes()); // DataSize
    res.extend_from_slice(&32u32.to_le_bytes()); // HeaderSize
    res.extend_from_slice(&[0xFF, 0xFF, 0x00, 0x00]); // Type = 0
    res.extend_from_slice(&[0xFF, 0xFF, 0x00, 0x00]); // Name = 0
    res.extend_from_slice(&0u32.to_le_bytes()); // DataVersion
    res.extend_from_slice(&0u16.to_le_bytes()); // MemoryFlags
    res.extend_from_slice(&0u16.to_le_bytes()); // LanguageId
    res.extend_from_slice(&0u32.to_le_bytes()); // Version
    res.extend_from_slice(&0u32.to_le_bytes()); // Characteristics

    let mut group = Vec::with_capacity(6 + count * 14);
    group.extend_from_slice(&0u16.to_le_bytes());
    group.extend_from_slice(&1u16.to_le_bytes());
    group.extend_from_slice(&(count as u16).to_le_bytes());

    for idx in 0..count {
        let dir_off = 6 + idx * 16;
        let width = ico[dir_off];
        let height = ico[dir_off + 1];
        let color_count = ico[dir_off + 2];
        let entry_reserved = ico[dir_off + 3];
        let raw_planes = read_u16_le(&ico, dir_off + 4);
        let raw_bit_count = read_u16_le(&ico, dir_off + 6);
        let bytes_in_res = read_u32_le(&ico, dir_off + 8) as usize;
        let image_offset = read_u32_le(&ico, dir_off + 12) as usize;

        assert!(
            image_offset + bytes_in_res <= ico.len(),
            "ICO entry {idx} extends past EOF"
        );
        let res_id = (idx + 1) as u16;
        let slice = &ico[image_offset..image_offset + bytes_in_res];

        // 0x1010 = MOVEABLE | PURE
        push_res_record(&mut res, RT_ICON, res_id, 0x1010, slice);

        let planes = if raw_planes == 0 { 1 } else { raw_planes };
        let bit_count = if raw_bit_count == 0 { 32 } else { raw_bit_count };

        group.push(width);
        group.push(height);
        group.push(color_count);
        group.push(entry_reserved);
        group.extend_from_slice(&planes.to_le_bytes());
        group.extend_from_slice(&bit_count.to_le_bytes());
        group.extend_from_slice(&(bytes_in_res as u32).to_le_bytes());
        group.extend_from_slice(&res_id.to_le_bytes());
    }

    // 0x1030 = MOVEABLE | PURE | DISCARDABLE
    push_res_record(&mut res, RT_GROUP_ICON, 1, 0x1030, &group);

    fs::write(res_path, res).expect("failed to write compiled .res file");
}

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=assets/PrettyWindows.ico");

    let target_os = env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    let target_env = env::var("CARGO_CFG_TARGET_ENV").unwrap_or_default();
    if target_os != "windows" || target_env != "msvc" {
        return;
    }

    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    let out_dir = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR"));
    let ico_path = manifest_dir.join("assets").join("PrettyWindows.ico");
    let res_path = out_dir.join("PrettyWindows.res");

    compile_ico_to_res(&ico_path, &res_path);
    println!("cargo:rustc-link-arg-bin=PrettyWindows={}", res_path.display());
}
