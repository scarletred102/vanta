use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

use vanta_image::{build_image, ImageContents, ImageOptions, RootFile};

const REDOXFS_REVISION: &str = "99bc185bf8ad8bd6f4d2562c424d800c2a3d310b";
const RUST_TOOLCHAIN: &str = "nightly-2026-07-10";
const ESP_SECTORS: u64 = 65_536;
const ROOT_SECTORS: u64 = 262_144;

fn main() -> ExitCode {
    match env::args().nth(1).as_deref() {
        Some("image") => match build_default_image() {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("xtask image: {error}");
                ExitCode::FAILURE
            }
        },
        _ => {
            eprintln!("usage: cargo xtask image");
            ExitCode::FAILURE
        }
    }
}

fn build_default_image() -> Result<(), String> {
    let root = workspace_root();
    build_kernel(&root)?;
    build_userland(&root)?;

    let boot_efi = read_file(root.join("esp/EFI/BOOT/BOOTX64.EFI"))?;
    let kernel = read_file(root.join("target/x86_64-unknown-none/release/vanta-kernel"))?;
    let limine_config = read_file(root.join("esp/limine.conf"))?;
    let init = read_file(root.join("target/x86_64-unknown-none/release/init"))?;
    let vsh = read_file(root.join("target/x86_64-unknown-none/release/vsh"))?;
    let echo = read_file(root.join("target/x86_64-unknown-none/release/echo"))?;
    let cat = read_file(root.join("target/x86_64-unknown-none/release/cat"))?;
    let true_program = read_file(root.join("target/x86_64-unknown-none/release/true"))?;
    let false_program = read_file(root.join("target/x86_64-unknown-none/release/false"))?;
    let root_files = [
        RootFile {
            path: "/sbin/init",
            contents: &init,
            mode: 0o755,
            uid: 0,
            gid: 0,
        },
        RootFile {
            path: "/bin/vsh",
            contents: &vsh,
            mode: 0o755,
            uid: 0,
            gid: 0,
        },
        RootFile {
            path: "/bin/echo",
            contents: &echo,
            mode: 0o755,
            uid: 0,
            gid: 0,
        },
        RootFile {
            path: "/bin/cat",
            contents: &cat,
            mode: 0o755,
            uid: 0,
            gid: 0,
        },
        RootFile {
            path: "/bin/true",
            contents: &true_program,
            mode: 0o755,
            uid: 0,
            gid: 0,
        },
        RootFile {
            path: "/bin/false",
            contents: &false_program,
            mode: 0o755,
            uid: 0,
            gid: 0,
        },
    ];
    let image = build_image(
        ImageOptions {
            esp_sectors: ESP_SECTORS,
            root_sectors: ROOT_SECTORS,
        },
        ImageContents {
            boot_efi: &boot_efi,
            kernel: &kernel,
            limine_config: &limine_config,
            root_files: &root_files,
        },
    )
    .map_err(|error| format!("image construction failed: {error:?}"))?;

    let output = root.join("target/vanta-gpt.img");
    fs::write(&output, image.bytes()).map_err(|error| format!("{}: {error}", output.display()))?;
    let manifest = output.with_extension("manifest");
    fs::write(
        &manifest,
        format!(
            "image-builder=vanta-image@{}\nkernel-revision={}\nredoxfs-revision={}\nroot-start-lba={}\nroot-sectors={}\n",
            env!("CARGO_PKG_VERSION"),
            git_revision(&root),
            REDOXFS_REVISION,
            image.root_partition().start_lba,
            image.root_partition().sector_count(),
        ),
    )
    .map_err(|error| format!("{}: {error}", manifest.display()))?;

    println!("[image] {}", output.display());
    println!("[image] {}", manifest.display());
    Ok(())
}

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("xtask is in the Rust workspace")
        .to_path_buf()
}

fn build_kernel(root: &Path) -> Result<(), String> {
    let rustup = env::var_os("RUSTUP").unwrap_or_else(|| {
        PathBuf::from(env::var_os("USERPROFILE").expect("USERPROFILE is set"))
            .join(".cargo/bin/rustup.exe")
            .into_os_string()
    });
    let status = Command::new(rustup)
        .current_dir(root)
        .args([
            "run",
            RUST_TOOLCHAIN,
            "cargo",
            "build",
            "-p",
            "vanta-kernel",
            "--target",
            "x86_64-unknown-none",
            "--release",
        ])
        .status()
        .map_err(|error| format!("failed to start kernel build: {error}"))?;
    status
        .success()
        .then_some(())
        .ok_or_else(|| format!("kernel build exited with {status}"))
}

fn build_userland(root: &Path) -> Result<(), String> {
    let rustup = rustup_path();
    let user_rustflags = [
        "-C link-arg=-Tuserland/linker.ld",
        "-C link-arg=-static",
        "-C link-arg=-nostdlib",
        "-C link-arg=-z",
        "-C link-arg=max-page-size=0x1000",
        "-C relocation-model=static",
    ]
    .join(" ");
    let status = Command::new(rustup)
        .current_dir(root)
        .env("RUSTFLAGS", user_rustflags)
        .args([
            "run",
            RUST_TOOLCHAIN,
            "cargo",
            "build",
            "-p",
            "vanta-userland",
            "--bins",
            "--target",
            "x86_64-unknown-none",
            "--release",
        ])
        .status()
        .map_err(|error| format!("failed to start userland build: {error}"))?;
    status
        .success()
        .then_some(())
        .ok_or_else(|| format!("userland build exited with {status}"))
}

fn rustup_path() -> std::ffi::OsString {
    env::var_os("RUSTUP").unwrap_or_else(|| {
        PathBuf::from(env::var_os("USERPROFILE").expect("USERPROFILE is set"))
            .join(".cargo/bin/rustup.exe")
            .into_os_string()
    })
}

fn read_file(path: PathBuf) -> Result<Vec<u8>, String> {
    fs::read(&path).map_err(|error| format!("{}: {error}", path.display()))
}

fn git_revision(root: &Path) -> String {
    Command::new("git")
        .current_dir(root)
        .args(["rev-parse", "HEAD"])
        .output()
        .ok()
        .filter(|output| output.status.success())
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .map(|revision| revision.trim().to_owned())
        .filter(|revision| !revision.is_empty())
        .unwrap_or_else(|| "unknown".to_owned())
}
