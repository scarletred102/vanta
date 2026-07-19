#![no_std]
#![no_main]
#![feature(abi_x86_interrupt)]
#![feature(alloc_error_handler)]

extern crate alloc;

use alloc::{boxed::Box, vec::Vec};
use core::alloc::Layout;
use core::panic::PanicInfo;
use limine::request::{FramebufferRequest, HhdmRequest, MemmapRequest, MpRequest};
use limine::{BaseRevision, RequestsEndMarker, RequestsStartMarker};

mod apic;
mod elf;
mod framebuffer;
mod fs;
mod gdt;
mod heap;
mod interrupts;
mod keyboard;
mod memory;
mod paging;
mod process;
mod scheduler;
mod serial;
mod shell;
mod smp;
mod storage;
mod syscall;
mod vfs;

#[used]
#[link_section = ".requests"]
static BASE_REVISION: BaseRevision = BaseRevision::with_revision(3);

#[used]
#[link_section = ".requests"]
static FRAMEBUFFER_REQUEST: FramebufferRequest = FramebufferRequest::new();

#[used]
#[link_section = ".requests"]
static MEMMAP_REQUEST: MemmapRequest = MemmapRequest::new();

#[used]
#[link_section = ".requests"]
static HHDM_REQUEST: HhdmRequest = HhdmRequest::new();

#[used]
#[link_section = ".requests"]
static MP_REQUEST: MpRequest = MpRequest::new(limine::mp::MP_FLAG_X2APIC);

#[used]
#[link_section = ".requests_start_marker"]
static REQUESTS_START: RequestsStartMarker = RequestsStartMarker::new();

#[used]
#[link_section = ".requests_end_marker"]
static REQUESTS_END: RequestsEndMarker = RequestsEndMarker::new();

#[no_mangle]
pub extern "C" fn _start() -> ! {
    serial::init();
    serial_println!("[boot] vanta kernel: limine entry");

    if !BASE_REVISION.is_supported() {
        serial_println!("[boot] WARNING: limine base revision unsupported");
    }

    if let Some(fb_resp) = FRAMEBUFFER_REQUEST.response() {
        let fbs = fb_resp.framebuffers();
        if let Some(fb) = fbs.first() {
            serial_println!(
                "[boot] framebuffer {}x{} pitch={} bpp={}",
                fb.width,
                fb.height,
                fb.pitch,
                fb.bpp
            );
            framebuffer::init(fb);
        } else {
            serial_println!("[boot] WARNING: limine returned 0 framebuffers");
        }
    } else {
        serial_println!("[boot] WARNING: no framebuffer response");
    }

    if let Some(memmap_resp) = MEMMAP_REQUEST.response() {
        let stats = memory::init(memmap_resp);
        serial_println!(
            "[mm] entries={} usable={} MiB frames={} tracked={}",
            stats.map_entries,
            stats.usable_bytes / (1024 * 1024),
            stats.usable_frames,
            stats.tracked_frames
        );

        let first = memory::alloc_frame();
        let second = memory::alloc_frame();
        match (first, second) {
            (Some(first), Some(second)) if first != second => {
                let first_freed = memory::free_frame(first);
                let second_freed = memory::free_frame(second);
                let recycled = memory::alloc_frame();
                let reuse_ok = recycled.is_some_and(|frame| frame == first || frame == second);
                if let Some(frame) = recycled {
                    let _ = memory::free_frame(frame);
                }
                if first_freed && second_freed && reuse_ok {
                    serial_println!(
                        "[mm] allocate/free/reuse self-check passed: {:#x}, {:#x}",
                        first.start_address(),
                        second.start_address()
                    );
                } else {
                    serial_println!("[mm] WARNING: frame allocator reuse self-check failed");
                }
            }
            _ => serial_println!("[mm] WARNING: frame allocator self-check failed"),
        }
    } else {
        serial_println!("[mm] WARNING: no Limine memory-map response");
    }

    if let Some(hhdm_resp) = HHDM_REQUEST.response() {
        paging::init(hhdm_resp.offset);
        let summary = paging::inspect_current();
        serial_println!(
            "[vm] hhdm={:#x} cr3={:#x} pml4-present={}",
            summary.hhdm_offset,
            summary.cr3,
            summary.present_pml4_entries
        );

        let hhdm_roundtrip = paging::phys_to_virt(0x2000)
            .and_then(paging::virt_to_phys)
            .is_some_and(|physical| physical == 0x2000);
        serial_println!(
            "[vm] HHDM round-trip self-check {}",
            if hhdm_roundtrip { "passed" } else { "failed" }
        );

        match paging::translate(_start as *const () as usize as u64) {
            Some(translation) => serial_println!(
                "[vm] page-table self-check passed: _start -> {:#x} ({:#x} page)",
                translation.physical_address,
                translation.page_size
            ),
            None => serial_println!("[vm] WARNING: _start translation is unmapped"),
        }

        match paging::create_address_space() {
            Ok(space) => match memory::alloc_frame() {
                Some(frame) => {
                    let test_virtual_address = 0x4000_0000;
                    let flags = paging::MAP_USER | paging::MAP_WRITABLE;
                    match paging::map(space, test_virtual_address, frame.start_address(), flags) {
                        Ok(()) => match paging::translate_in(space, test_virtual_address) {
                            Some(mapping) if mapping.physical_address == frame.start_address() => {
                                let refused_live_destroy = matches!(
                                    paging::destroy_address_space(space),
                                    Err(paging::MapError::MappingsRemain)
                                );
                                match paging::unmap(space, test_virtual_address) {
                                    Ok(Some(unmapped)) if unmapped == frame.start_address() => {
                                        let frame_reused = memory::free_frame(frame);
                                        let tables_freed = paging::destroy_address_space(space);
                                        if refused_live_destroy
                                            && frame_reused
                                            && tables_freed.is_ok()
                                        {
                                            serial_println!(
                                                "[vm] address-space lifecycle self-check passed: pml4={:#x} tables-freed={}",
                                                space.pml4_phys,
                                                tables_freed.unwrap_or(0)
                                            );
                                        } else {
                                            serial_println!(
                                                "[vm] WARNING: address-space cleanup self-check failed"
                                            );
                                        }
                                    }
                                    _ => serial_println!("[vm] WARNING: unmap self-check failed"),
                                }
                            }
                            _ => serial_println!("[vm] WARNING: mapped address translation failed"),
                        },
                        Err(error) => {
                            serial_println!("[vm] WARNING: map self-check failed: {:?}", error)
                        }
                    }
                }
                None => serial_println!("[vm] WARNING: no frame for map self-check"),
            },
            Err(error) => {
                serial_println!("[vm] WARNING: address-space creation failed: {:?}", error)
            }
        }

        match heap::init() {
            Ok(heap_stats) => {
                let mut values = Vec::with_capacity(256);
                for index in 0..256u64 {
                    values.push((index * 3) ^ 0x5a);
                }
                let boxed_value = Box::new(0xfeed_beef_u64);
                let values_ok = values
                    .iter()
                    .enumerate()
                    .all(|(index, value)| *value == (index as u64 * 3) ^ 0x5a);
                let boxed_ok = *boxed_value == 0xfeed_beef_u64;
                let peak_used = heap::stats().used;
                drop(boxed_value);
                drop(values);
                let reclaimed = heap::stats().free == heap_stats.size;
                if values_ok && boxed_ok && reclaimed {
                    serial_println!(
                        "[heap] allocation/reclaim self-check passed: base={:#x} size={} peak-used={} free={}",
                        heap_stats.base,
                        heap_stats.size,
                        peak_used,
                        heap::stats().free
                    );
                } else {
                    serial_println!("[heap] WARNING: allocation/reclaim self-check failed");
                }
            }
            Err(error) => {
                serial_println!("[heap] WARNING: heap initialization failed: {:?}", error)
            }
        }
    } else {
        serial_println!("[vm] WARNING: no Limine HHDM response");
    }

    let storage_ready = vfs::initialize_root(128)
        .and_then(|()| vfs::write_root("/etc/config", b"vanta-storage"))
        .and_then(|()| vfs::read_root("/etc/config"))
        .map(|bytes| bytes == b"vanta-storage")
        .unwrap_or(false)
        && vfs::write_root("/etc/config", b"vanta-vfs-syscall\n").is_ok()
        && vfs::remount_root().is_ok()
        && vfs::read_root("/etc/config").ok().as_deref() == Some(b"vanta-vfs-syscall\n");
    if storage_ready {
        serial_println!("[storage] writable VFS root mounted and persistence self-check passed");
    } else {
        serial_println!("[storage] WARNING: block device/VFS self-check failed");
    }

    let init_image = match fs::FileSystem::new(&fs::INITRAMFS) {
        Ok(rootfs) => {
            let init = rootfs.read("/bin/init");
            let motd = rootfs.read("/etc/motd");
            let bin_directory = rootfs.is_directory("/bin");
            let etc_directory = rootfs.is_directory("/etc");
            let init_executable = rootfs.is_executable("/bin/init");
            let missing = rootfs.read("/missing");
            let directory_as_file = rootfs.read("/bin");
            let traversal = rootfs.read("/bin/../etc/motd");
            let truncated = fs::FileSystem::new(&fs::INITRAMFS[..fs::INITRAMFS.len() - 1]);
            match (
                init,
                motd,
                bin_directory,
                etc_directory,
                init_executable,
                missing,
                directory_as_file,
                traversal,
                truncated,
            ) {
                (
                    Ok(init),
                    Ok(motd),
                    Ok(true),
                    Ok(true),
                    Ok(true),
                    Err(fs::FsError::NotFound),
                    Err(fs::FsError::NotFile),
                    Err(fs::FsError::InvalidPath),
                    Err(fs::FsError::Truncated),
                ) if init.len() == elf::TEST_ELF.len() && motd == b"Vanta initramfs\n" => {
                    serial_println!(
                        "[fs] initramfs self-check passed: entries={} init-bytes={} motd-bytes={}",
                        rootfs.entry_count(),
                        init.len(),
                        motd.len()
                    );
                    init
                }
                _ => {
                    serial_println!("[fs] WARNING: initramfs self-check failed");
                    &[]
                }
            }
        }
        Err(error) => {
            serial_println!("[fs] WARNING: initramfs parse failed: {:?}", error);
            &[]
        }
    };

    kprintln!("vanta os | kernel terminal");
    kprintln!("-----------------------------------");

    gdt::init();
    kprintln!("[ok] gdt");
    serial_println!("[boot] gdt loaded");

    let apic = apic::initialize();
    serial_println!(
        "[apic] mode={:?} lapic-id={} base={:#x} x2apic-supported={}",
        apic.mode,
        apic.lapic_id,
        apic.physical_base,
        apic.x2apic_supported
    );

    let smp = smp::bootstrap(MP_REQUEST.response());
    serial_println!(
        "[smp] reported-cpus={} bsp-lapic={} requested-aps={} online-aps={} x2apic={}",
        smp.reported_cpus,
        smp.bsp_lapic_id,
        smp.requested_aps,
        smp.online_aps,
        smp.x2apic_enabled
    );

    interrupts::init_idt();
    kprintln!("[ok] idt");
    serial_println!("[boot] idt loaded");

    let syscall_ready = syscall::init();
    if syscall_ready {
        serial_println!("[boot] syscall ABI configured");
    } else {
        serial_println!("[boot] WARNING: syscall ABI configuration failed");
    }

    unsafe {
        let mut pics = interrupts::PICS.lock();
        pics.initialize();
        // unmask IRQ0 (timer) and IRQ1 (keyboard); mask the rest
        pics.write_masks(0b1111_1100, 0b1111_1111);
    }
    if interrupts::initialize_timer(100) {
        serial_println!("[boot] PIT configured: 100 Hz");
    } else {
        serial_println!("[boot] WARNING: PIT configuration failed");
    }
    x86_64::instructions::interrupts::enable();
    kprintln!("[ok] pic + sti");
    serial_println!("[boot] interrupts enabled");

    match process::load_elf(init_image) {
        Ok(mut process) => {
            let entry = process.entry();
            let entry_translation = paging::translate_in(process.address_space(), entry);
            let entry_flags = paging::flags_in(process.address_space(), entry).unwrap_or(0);
            let stack_flags = paging::flags_in(
                process.address_space(),
                process.user_stack_top() - memory::PAGE_SIZE,
            )
            .unwrap_or(0);
            let user = entry_flags & paging::MAP_USER != 0;
            let executable = entry_flags & paging::MAP_NO_EXECUTE == 0;
            let stack_user = stack_flags & paging::MAP_USER != 0;
            let stack_no_execute = stack_flags & paging::MAP_NO_EXECUTE != 0;
            let data_flags =
                paging::flags_in(process.address_space(), elf::TEST_DATA_ADDRESS).unwrap_or(0);
            let data_user = data_flags & paging::MAP_USER != 0;
            let data_writable = data_flags & paging::MAP_WRITABLE != 0;
            let data_no_execute = data_flags & paging::MAP_NO_EXECUTE != 0;
            let data_initialized = process.read_user_byte(elf::TEST_DATA_ADDRESS) == Some(b'/');
            let data_bss_zero = process.read_user_byte(elf::TEST_DATA_ADDRESS + 11) == Some(0);
            if entry_translation.is_some()
                && user
                && executable
                && stack_user
                && stack_no_execute
                && data_user
                && data_writable
                && data_no_execute
                && data_initialized
                && data_bss_zero
            {
                match process.destroy() {
                    Ok(freed_tables) => serial_println!(
                        "[proc] ELF load/cleanup self-check passed: entry={:#x} tables-freed={}",
                        entry,
                        freed_tables
                    ),
                    Err(error) => serial_println!("[proc] cleanup self-check failed: {:?}", error),
                }
            } else {
                serial_println!("[proc] ELF permission self-check failed");
            }
        }
        Err(error) => serial_println!("[proc] ELF load self-check failed: {:?}", error),
    }

    if syscall_ready {
        match (process::load_elf(init_image), process::load_elf(init_image)) {
            (Ok(first), Ok(second)) => {
                let mut tasks = Vec::with_capacity(2);
                tasks.push(Box::new(first));
                tasks.push(Box::new(second));
                unsafe { scheduler::start(tasks) }
            }
            (Err(error), _) | (_, Err(error)) => {
                serial_println!("[sched] user-task load failed: {:?}", error)
            }
        }
    }

    shell::run()
}

#[alloc_error_handler]
fn alloc_error(layout: Layout) -> ! {
    serial_println!(
        "[heap] allocation failed: size={} align={}",
        layout.size(),
        layout.align()
    );
    loop {
        x86_64::instructions::hlt();
    }
}

#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    serial_println!("[PANIC] {}", info);
    loop {
        x86_64::instructions::hlt();
    }
}
