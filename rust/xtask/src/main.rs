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
        Some("sdk") => {
            let root = workspace_root();
            match build_sdk(&root) {
                Ok(()) => ExitCode::SUCCESS,
                Err(error) => {
                    eprintln!("xtask sdk: {error}");
                    ExitCode::FAILURE
                }
            }
        }
        _ => {
            eprintln!("usage: cargo xtask image|sdk");
            ExitCode::FAILURE
        }
    }
}

fn build_sdk(root: &Path) -> Result<(), String> {
    let rustup = rustup_path();
    let status = Command::new(&rustup)
        .current_dir(&root)
        .args([
            "run",
            RUST_TOOLCHAIN,
            "cargo",
            "build",
            "-p",
            "libvanta",
            "--target",
            "x86_64-unknown-none",
            "--release",
        ])
        .status()
        .map_err(|error| format!("failed to start SDK build: {error}"))?;
    if !status.success() {
        return Err(format!("SDK build exited with {status}"));
    }

    let output = root.join("target/sdk");
    fs::create_dir_all(output.join("include"))
        .map_err(|error| format!("{}: {error}", output.display()))?;
    let library = root.join("target/x86_64-unknown-none/release/liblibvanta.a");
    fs::copy(&library, output.join("libvanta.a"))
        .map_err(|error| format!("{}: {error}", library.display()))?;
    let header = root.join("libvanta/include/vanta.h");
    fs::copy(&header, output.join("include/vanta.h"))
        .map_err(|error| format!("{}: {error}", header.display()))?;
    fs::copy(
        root.join("libvanta/examples/hello.c"),
        output.join("hello.c"),
    )
    .map_err(|error| format!("SDK sample: {error}"))?;
    fs::copy(
        root.join("libvanta/examples/sdk_smoke.c"),
        output.join("sdk_smoke.c"),
    )
    .map_err(|error| format!("SDK smoke sample: {error}"))?;
    fs::copy(
        root.join("libvanta/examples/stdio_smoke.c"),
        output.join("stdio_smoke.c"),
    )
    .map_err(|error| format!("SDK stdio sample: {error}"))?;
    fs::copy(
        root.join("libvanta/examples/dir_smoke.c"),
        output.join("dir_smoke.c"),
    )
    .map_err(|error| format!("SDK directory sample: {error}"))?;
    fs::write(
        output.join("manifest.txt"),
        format!(
            "sdk=libvanta@{}\nabi-version=0\ntarget=x86_64-unknown-none\nsource-revision={}\n",
            env!("CARGO_PKG_VERSION"),
            git_revision(&root),
        ),
    )
    .map_err(|error| format!("SDK manifest: {error}"))?;
    compile_c_sample(&root)?;
    println!("[sdk] {}", output.display());
    Ok(())
}

fn compile_c_sample(root: &Path) -> Result<(), String> {
    compile_c_program(root, "hello.c", "hello.o", "hello-vanta.elf")?;
    compile_c_program(root, "sdk_smoke.c", "sdk_smoke.o", "sdk-smoke-vanta.elf")?;
    compile_c_program(
        root,
        "stdio_smoke.c",
        "stdio_smoke.o",
        "stdio-smoke-vanta.elf",
    )?;
    compile_c_program(root, "dir_smoke.c", "dir_smoke.o", "dir-smoke-vanta.elf")
}

fn compile_c_program(
    root: &Path,
    source: &str,
    object: &str,
    executable: &str,
) -> Result<(), String> {
    let source_path = format!("target/sdk/{source}");
    let object_path = format!("target/sdk/{object}");
    let executable_path = format!("target/sdk/{executable}");
    let status = Command::new("zig")
        .current_dir(root)
        .args([
            "cc",
            "-target",
            "x86_64-freestanding",
            "-ffreestanding",
            "-fno-sanitize=undefined",
            "-fno-stack-protector",
            "-nostdlib",
            "-I",
            "libvanta/include",
            "-c",
            source_path.as_str(),
            "-o",
            object_path.as_str(),
        ])
        .status()
        .map_err(|error| {
            format!("failed to start C SDK compiler for {source} (zig cc): {error}")
        })?;
    if !status.success() {
        return Err(format!(
            "C SDK compilation for {source} exited with {status}"
        ));
    }
    let status = Command::new("zig")
        .current_dir(root)
        .args([
            "cc",
            "-target",
            "x86_64-freestanding",
            "-nostdlib",
            "-fuse-ld=lld",
            "-Wl,-T,userland/linker.ld",
            object_path.as_str(),
            "target/sdk/libvanta.a",
            "-o",
            executable_path.as_str(),
        ])
        .status()
        .map_err(|error| format!("failed to start C SDK linker for {source} (zig cc): {error}"))?;
    if !status.success() {
        return Err(format!("C SDK linking for {source} exited with {status}"));
    }
    Ok(())
}

fn build_default_image() -> Result<(), String> {
    let root = workspace_root();
    build_kernel(&root)?;
    build_userland(&root)?;
    build_sdk(&root)?;

    let boot_efi = read_file(root.join("esp/EFI/BOOT/BOOTX64.EFI"))?;
    let kernel = read_file(root.join("target/x86_64-unknown-none/release/vanta-kernel"))?;
    let limine_config = read_file(root.join("esp/limine.conf"))?;
    let init = read_file(root.join("target/x86_64-unknown-none/release/init"))?;
    let vsh = read_file(root.join("target/x86_64-unknown-none/release/vsh"))?;
    let echo = read_file(root.join("target/x86_64-unknown-none/release/echo"))?;
    let cat = read_file(root.join("target/x86_64-unknown-none/release/cat"))?;
    let true_program = read_file(root.join("target/x86_64-unknown-none/release/true"))?;
    let false_program = read_file(root.join("target/x86_64-unknown-none/release/false"))?;
    let native_gate = read_file(root.join("target/x86_64-unknown-none/release/native-gate"))?;
    let ls = read_file(root.join("target/x86_64-unknown-none/release/ls"))?;
    let mkdir = read_file(root.join("target/x86_64-unknown-none/release/mkdir"))?;
    let rm = read_file(root.join("target/x86_64-unknown-none/release/rm"))?;
    let mv = read_file(root.join("target/x86_64-unknown-none/release/mv"))?;
    let pwd = read_file(root.join("target/x86_64-unknown-none/release/pwd"))?;
    let stat = read_file(root.join("target/x86_64-unknown-none/release/stat"))?;
    let c_hello = read_file(root.join("target/sdk/hello-vanta.elf"))?;
    let c_sdk_smoke = read_file(root.join("target/sdk/sdk-smoke-vanta.elf"))?;
    let c_stdio_smoke = read_file(root.join("target/sdk/stdio-smoke-vanta.elf"))?;
    let c_dir_smoke = read_file(root.join("target/sdk/dir-smoke-vanta.elf"))?;
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
        RootFile {
            path: "/bin/native-gate",
            contents: &native_gate,
            mode: 0o755,
            uid: 0,
            gid: 0,
        },
        RootFile {
            path: "/bin/ls",
            contents: &ls,
            mode: 0o755,
            uid: 0,
            gid: 0,
        },
        RootFile {
            path: "/bin/mkdir",
            contents: &mkdir,
            mode: 0o755,
            uid: 0,
            gid: 0,
        },
        RootFile {
            path: "/bin/rm",
            contents: &rm,
            mode: 0o755,
            uid: 0,
            gid: 0,
        },
        RootFile {
            path: "/bin/mv",
            contents: &mv,
            mode: 0o755,
            uid: 0,
            gid: 0,
        },
        RootFile {
            path: "/bin/pwd",
            contents: &pwd,
            mode: 0o755,
            uid: 0,
            gid: 0,
        },
        RootFile {
            path: "/bin/stat",
            contents: &stat,
            mode: 0o755,
            uid: 0,
            gid: 0,
        },
        RootFile {
            path: "/bin/c-hello",
            contents: &c_hello,
            mode: 0o755,
            uid: 0,
            gid: 0,
        },
        RootFile {
            path: "/bin/c-sdk-smoke",
            contents: &c_sdk_smoke,
            mode: 0o755,
            uid: 0,
            gid: 0,
        },
        RootFile {
            path: "/bin/c-stdio-smoke",
            contents: &c_stdio_smoke,
            mode: 0o755,
            uid: 0,
            gid: 0,
        },
        RootFile {
            path: "/bin/c-dir-smoke",
            contents: &c_dir_smoke,
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
