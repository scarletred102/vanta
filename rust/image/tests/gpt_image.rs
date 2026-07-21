use std::io::Cursor;

use vanta_gpt::discover_vanta_root;
use vanta_image::{build_image, ImageContents, ImageOptions};

#[test]
fn builds_a_gpt_disk_with_bootable_esp_and_redoxfs_root() {
    let image = build_image(
        ImageOptions {
            esp_sectors: 8_192,
            root_sectors: 32_768,
        },
        ImageContents {
            boot_efi: b"limine-efi",
            kernel: b"vanta-kernel",
            limine_config: b"/vanta\n",
        },
    )
    .expect("GPT image");

    let root = discover_vanta_root(|sector, buffer| {
        let start = sector as usize * 512;
        buffer.copy_from_slice(&image.bytes()[start..start + 512]);
        Ok(())
    })
    .expect("Vanta root partition");

    assert_eq!(root, image.root_partition());
    assert_eq!(image.root_bytes()[..8], *b"RedoxFS\0");

    let esp = fatfs::FileSystem::new(
        Cursor::new(image.esp_bytes().to_vec()),
        fatfs::FsOptions::new(),
    )
    .expect("FAT ESP");
    let root_dir = esp.root_dir();
    assert!(root_dir.open_file("EFI/BOOT/BOOTX64.EFI").is_ok());
    assert!(root_dir.open_file("boot/vanta-kernel").is_ok());
}
