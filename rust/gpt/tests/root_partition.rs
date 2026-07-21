use vanta_gpt::{discover_vanta_root, find_vanta_root, VANTA_ROOT_TYPE_GUID};

const HEADER_SIZE: usize = 92;
const ENTRY_SIZE: usize = 128;

#[test]
fn finds_a_valid_vanta_root_partition() {
    let mut header = [0_u8; 512];
    let mut entries = [0_u8; ENTRY_SIZE];
    header[..8].copy_from_slice(b"EFI PART");
    put_u32(&mut header, 12, HEADER_SIZE as u32);
    put_u64(&mut header, 24, 1);
    put_u64(&mut header, 32, 4095);
    put_u64(&mut header, 40, 34);
    put_u64(&mut header, 48, 4062);
    put_u64(&mut header, 72, 2);
    put_u32(&mut header, 80, 1);
    put_u32(&mut header, 84, ENTRY_SIZE as u32);

    entries[..16].copy_from_slice(&VANTA_ROOT_TYPE_GUID);
    put_u64(&mut entries, 32, 2048);
    put_u64(&mut entries, 40, 4062);
    put_u32(&mut header, 88, crc32(&entries));
    let checksum = header_crc(&header);
    put_u32(&mut header, 16, checksum);

    let root = find_vanta_root(&header, &entries).expect("valid root partition");

    assert_eq!(root.start_lba, 2048);
    assert_eq!(root.sector_count(), 2015);
    assert_eq!(root.absolute_lba(0), Some(2048));
    assert_eq!(root.absolute_lba(2014), Some(4062));
    assert_eq!(root.absolute_lba(2015), None);
}

#[test]
fn rejects_a_root_entry_outside_the_usable_range() {
    let mut header = [0_u8; 512];
    let mut entries = [0_u8; ENTRY_SIZE];
    header[..8].copy_from_slice(b"EFI PART");
    put_u32(&mut header, 12, HEADER_SIZE as u32);
    put_u64(&mut header, 24, 1);
    put_u64(&mut header, 32, 4095);
    put_u64(&mut header, 40, 34);
    put_u64(&mut header, 48, 4062);
    put_u64(&mut header, 72, 2);
    put_u32(&mut header, 80, 1);
    put_u32(&mut header, 84, ENTRY_SIZE as u32);

    entries[..16].copy_from_slice(&VANTA_ROOT_TYPE_GUID);
    put_u64(&mut entries, 32, 33);
    put_u64(&mut entries, 40, 4062);
    put_u32(&mut header, 88, crc32(&entries));
    let checksum = header_crc(&header);
    put_u32(&mut header, 16, checksum);

    assert!(find_vanta_root(&header, &entries).is_err());
}

#[test]
fn discovers_a_root_partition_from_its_gpt_sectors() {
    let mut header = [0_u8; 512];
    let mut entries = [0_u8; ENTRY_SIZE];
    header[..8].copy_from_slice(b"EFI PART");
    put_u32(&mut header, 12, HEADER_SIZE as u32);
    put_u64(&mut header, 24, 1);
    put_u64(&mut header, 32, 4095);
    put_u64(&mut header, 40, 34);
    put_u64(&mut header, 48, 4062);
    put_u64(&mut header, 72, 2);
    put_u32(&mut header, 80, 1);
    put_u32(&mut header, 84, ENTRY_SIZE as u32);
    entries[..16].copy_from_slice(&VANTA_ROOT_TYPE_GUID);
    put_u64(&mut entries, 32, 2048);
    put_u64(&mut entries, 40, 4062);
    put_u32(&mut header, 88, crc32(&entries));
    let checksum = header_crc(&header);
    put_u32(&mut header, 16, checksum);

    let root = discover_vanta_root(|sector, buffer| match sector {
        1 => {
            buffer.copy_from_slice(&header);
            Ok(())
        }
        2 => {
            buffer[..ENTRY_SIZE].copy_from_slice(&entries);
            Ok(())
        }
        _ => Err(()),
    })
    .expect("root from GPT sectors");

    assert_eq!(root.start_lba, 2048);
}

fn put_u32(bytes: &mut [u8], offset: usize, value: u32) {
    bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}

fn put_u64(bytes: &mut [u8], offset: usize, value: u64) {
    bytes[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
}

fn header_crc(header: &[u8; 512]) -> u32 {
    let mut copy = *header;
    copy[16..20].fill(0);
    crc32(&copy[..HEADER_SIZE])
}

fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = !0_u32;
    for &byte in bytes {
        crc ^= byte as u32;
        for _ in 0..8 {
            crc = if crc & 1 == 0 {
                crc >> 1
            } else {
                (crc >> 1) ^ 0xedb8_8320
            };
        }
    }
    !crc
}
