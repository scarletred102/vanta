// ============================================================================
// VantaOS — Symmetric Multiprocessing (Phase 6)
// ============================================================================

const std = @import("std");
const limine = @import("../../limine.zig");
const serial = @import("serial.zig");
const vmm = @import("../../mm/vmm.zig");
const pmm = @import("../../mm/pmm.zig");
const cpu_local = @import("cpu_local.zig");
const interrupts = @import("interrupts.zig");
const idt = @import("idt.zig");
const gdt = @import("gdt.zig");
const tss = @import("tss.zig");
const sched = @import("../../sched/scheduler.zig");

pub const cpu_topology = struct {
    apic_ids: [64]u8 = [_]u8{0} ** 64,
    count: u8 = 0,
};

pub var topology: cpu_topology = .{};
pub var ap_ready_flag: u32 = 0;

comptime {
    asm (
        \\.code16
        \\.global ap_trampoline_start
        \\ap_trampoline_start:
        \\    cli
        \\    movw $0x3F8, %dx
        \\    movb $'R', %al
        \\    outb %al, %dx
        \\    xorw %ax, %ax
        \\    movw %ax, %ds
        \\    movw %ax, %es
        \\    movw %ax, %ss
        \\
        \\    lgdtl (ap_gdt_desc - ap_trampoline_start + 0x8000)
        \\
        \\    movl %cr0, %eax
        \\    orl $1, %eax
        \\    movl %eax, %cr0
        \\
        \\    ljmpl $0x08, $(ap_trampoline_32 - ap_trampoline_start + 0x8000)
        \\
        \\.code32
        \\ap_trampoline_32:
        \\    movw $0x3F8, %dx
        \\    movb $'T', %al
        \\    outb %al, %dx
        \\    movw $0x10, %ax
        \\    movw %ax, %ds
        \\    movw %ax, %es
        \\    movw %ax, %ss
        \\
        \\    movl %cr4, %eax
        \\    orl $0x20, %eax
        \\    movl %eax, %cr4
        \\
        \\    movl $0x7FF0, %eax
        \\    movl (%eax), %ebx
        \\    movl %ebx, %cr3
        \\
        \\    movl $0xC0000080, %ecx
        \\    rdmsr
        \\    orl $0x900, %eax
        \\    wrmsr
        \\
        \\    movl %cr0, %eax
        \\    orl $0x80000000, %eax
        \\    movl %eax, %cr0
        \\
        \\    ljmpl $0x18, $(ap_trampoline_64 - ap_trampoline_start + 0x8000)
        \\
        \\.code64
        \\ap_trampoline_64:
        \\    movw $0x3F8, %dx
        \\    movb $'L', %al
        \\    outb %al, %dx
        \\    xorw %ax, %ax
        \\    movw %ax, %ds
        \\    movw %ax, %es
        \\    movw %ax, %ss
        \\    movw %ax, %fs
        \\    movw %ax, %gs
        \\
        \\    // Enable SSE for compiler vector moves
        \\    movq %cr0, %rax
        \\    andq $-5, %rax
        \\    orq $2, %rax
        \\    movq %rax, %cr0
        \\    movq %cr4, %rax
        \\    orq $0x600, %rax
        \\    movq %rax, %cr4
        \\
        \\    movabs $0x7FE8, %rax
        \\    movq (%rax), %rdi
        \\
        \\    movabs $0x7FF8, %rax
        \\    movq (%rax), %rsp
        \\    subq $8, %rsp
        \\
        \\    movabs $ap_startup, %rax
        \\    jmp *%rax
        \\
        \\.align 4
        \\ap_gdt:
        \\    .quad 0
        \\    .quad 0x00CF9A000000FFFF
        \\    .quad 0x00CF92000000FFFF
        \\    .quad 0x00209A0000000000
        \\    .quad 0x0000920000000000
        \\
        \\ap_gdt_desc:
        \\    .word 39
        \\    .long (ap_gdt - ap_trampoline_start + 0x8000)
        \\
        \\ap_gdt_desc_64:
        \\    .word 39
        \\    .quad (ap_gdt - ap_trampoline_start + 0x8000)
        \\
        \\.global ap_trampoline_end
        \\ap_trampoline_end:
    );
}

extern const ap_trampoline_start: u8;
extern const ap_trampoline_end: u8;

pub fn smp_init(rsdp_phys: u64) void {
    serial.puts("[SMP]   Symmetric Multiprocessing initializing...\n");

    discover_cpus(rsdp_phys);

    prepare_trampoline();

    boot_aps();
}

fn discover_cpus(rsdp_phys: u64) void {
    const rsdp_virt = vmm.phys2virt(rsdp_phys);
    const sig = @as([*]const u8, @ptrFromInt(rsdp_virt));
    if (!std.mem.eql(u8, sig[0..8], "RSD PTR ")) {
        serial.puts("[SMP]   ACPI RSDP signature invalid. Falling back to 1 CPU.\n");
        return;
    }
    const revision = sig[15];
    var xsdt_phys: u64 = 0;
    var rsdt_phys: u64 = 0;
    if (revision >= 2) {
        xsdt_phys = @as(*align(1) const u64, @ptrFromInt(rsdp_virt + 24)).*;
    } else {
        rsdt_phys = @as(*align(1) const u32, @ptrFromInt(rsdp_virt + 16)).*;
    }

    const table_phys = if (xsdt_phys != 0) xsdt_phys else rsdt_phys;
    if (table_phys == 0) return;

    const table_virt = vmm.phys2virt(table_phys);
    const header_sig = @as([*]const u8, @ptrFromInt(table_virt));
    const header_len = @as(*align(1) const u32, @ptrFromInt(table_virt + 4)).*;

    const is_xsdt = (xsdt_phys != 0);
    var madt_phys: u64 = 0;

    if (is_xsdt) {
        if (!std.mem.eql(u8, header_sig[0..4], "XSDT")) return;
        const entry_count = (header_len - 36) / 8;
        const entries = @as([*]align(1) const u64, @ptrFromInt(table_virt + 36));
        var i: usize = 0;
        while (i < entry_count) : (i += 1) {
            const entry_virt = vmm.phys2virt(entries[i]);
            if (std.mem.eql(u8, @as([*]const u8, @ptrFromInt(entry_virt))[0..4], "APIC")) {
                madt_phys = entries[i];
                break;
            }
        }
    } else {
        if (!std.mem.eql(u8, header_sig[0..4], "RSDT")) return;
        const entry_count = (header_len - 36) / 4;
        const entries = @as([*]align(1) const u32, @ptrFromInt(table_virt + 36));
        var i: usize = 0;
        while (i < entry_count) : (i += 1) {
            const entry_virt = vmm.phys2virt(entries[i]);
            if (std.mem.eql(u8, @as([*]const u8, @ptrFromInt(entry_virt))[0..4], "APIC")) {
                madt_phys = entries[i];
                break;
            }
        }
    }

    if (madt_phys == 0) return;

    const madt_virt = vmm.phys2virt(madt_phys);
    const len = @as(*align(1) const u32, @ptrFromInt(madt_virt + 4)).*;
    
    topology.count = 0;

    var offset: usize = 44;
    while (offset < len) {
        const entry_type = @as(*const u8, @ptrFromInt(madt_virt + offset)).*;
        const entry_len = @as(*const u8, @ptrFromInt(madt_virt + offset + 1)).*;
        if (entry_len == 0) break;

        if (entry_type == 0) {
            const apic_id = @as(*const u8, @ptrFromInt(madt_virt + offset + 3)).*;
            const flags = @as(*align(1) const u32, @ptrFromInt(madt_virt + offset + 4)).*;
            if ((flags & 1) != 0) {
                if (topology.count < 64) {
                    topology.apic_ids[topology.count] = apic_id;
                    topology.count += 1;
                }
            }
        }
        offset += entry_len;
    }

    serial.puts("[SMP]   MADT Discovered CPU count: ");
    serial.putDec(topology.count);
    serial.puts("\n");
}

fn prepare_trampoline() void {
    const src_start = @intFromPtr(&ap_trampoline_start);
    const src_end = @intFromPtr(&ap_trampoline_end);
    const size = src_end - src_start;

    serial.puts("[SMP]   Trampoline size: ");
    serial.putDec(size);
    serial.puts(" bytes\n");

    const dest_virt = vmm.phys2virt(0x8000);
    const dest_slice = @as([*]u8, @ptrFromInt(dest_virt))[0..size];
    const src_slice = @as([*]const u8, @ptrFromInt(src_start))[0..size];
    @memcpy(dest_slice, src_slice);
}

fn boot_aps() void {
    serial.puts("[SMP]   boot_aps starting...\n");
    const bsp_apic_id = @as(u8, @truncate(interrupts.lapicRead(0x20) >> 24));
    serial.puts("[SMP]   BSP APIC ID: ");
    serial.putDec(bsp_apic_id);
    serial.puts("\n");
    
    // Set up BSP's own CpuLocal
    const bsp_cpu = &cpu_local.cpus[0];
    bsp_cpu.cpu_id = 0;
    bsp_cpu.apic_id = bsp_apic_id;
    bsp_cpu.self_ptr = @intFromPtr(bsp_cpu);
    bsp_cpu.lapic_base = 0xfee00000;
    bsp_cpu.tss_ptr = &tss.tss;
    
    // Configure GS Base MSR for BSP
    const MSR_GS_BASE: u32 = 0xC0000101;
    const MSR_KERNEL_GS_BASE: u32 = 0xC0000102;
    
    const lo: u32 = @truncate(@intFromPtr(bsp_cpu) & 0xFFFFFFFF);
    const hi: u32 = @truncate(@intFromPtr(bsp_cpu) >> 32);
    asm volatile ("wrmsr" : : [msr] "{ecx}" (MSR_GS_BASE), [lo] "{eax}" (lo), [hi] "{edx}" (hi));
    asm volatile ("wrmsr" : : [msr] "{ecx}" (MSR_KERNEL_GS_BASE), [lo] "{eax}" (lo), [hi] "{edx}" (hi));

    serial.puts("[SMP]   BSP GS Base MSR configured\n");


    // Share PML4 through mailbox
    const pml4_mailbox = @as(*volatile u64, @ptrFromInt(vmm.phys2virt(0x7FF0)));
    pml4_mailbox.* = vmm.AddressSpace.current().pml4_phys;

    serial.puts("[SMP]   PML4 mailbox written: 0x");
    serial.putHex(pml4_mailbox.*);
    serial.puts("\n");

    // Clear the NX bit at all page table levels for 0x7000 and 0x8000 to allow AP trampoline execution
    const pml4_phys_val = vmm.AddressSpace.current().pml4_phys;
    const pml4 = @as([*]volatile u64, @ptrFromInt(vmm.phys2virt(pml4_phys_val)));
    pml4[0] &= ~(@as(u64, 1) << 63);
    if ((pml4[0] & 1) != 0) {
        const pdpt_phys = pml4[0] & 0x000FFFFFFFFFF000;
        const pdpt = @as([*]volatile u64, @ptrFromInt(vmm.phys2virt(pdpt_phys)));
        pdpt[0] &= ~(@as(u64, 1) << 63);
        if ((pdpt[0] & 1) != 0) {
            const pd_phys = pdpt[0] & 0x000FFFFFFFFFF000;
            const pd = @as([*]volatile u64, @ptrFromInt(vmm.phys2virt(pd_phys)));
            pd[0] &= ~(@as(u64, 1) << 63);
            if ((pd[0] & 1) != 0) {
                const pt_phys = pd[0] & 0x000FFFFFFFFFF000;
                const pt = @as([*]volatile u64, @ptrFromInt(vmm.phys2virt(pt_phys)));
                pt[7] &= ~(@as(u64, 1) << 63); // 0x7000
                pt[8] &= ~(@as(u64, 1) << 63); // 0x8000
            }
        }
    }

    // Explicitly identity map trampoline and mailbox pages (0x7000 and 0x8000)
    _ = vmm.map(vmm.AddressSpace.current(), 0x7000, 0x7000, vmm.PTE_WRITE);
    _ = vmm.map(vmm.AddressSpace.current(), 0x8000, 0x8000, vmm.PTE_WRITE);
    serial.puts("[SMP]   Explicit identity map of 0x7000 and 0x8000 created\n");

    var i: usize = 0;
    while (i < topology.count) : (i += 1) {
        const apic_id = topology.apic_ids[i];
        serial.puts("[SMP]   Checking CPU ");
        serial.putDec(i);
        serial.puts(" with APIC ID ");
        serial.putDec(apic_id);
        serial.puts("\n");
        if (apic_id == bsp_apic_id) {
            serial.puts("[SMP]   Skip BSP\n");
            continue;
        }

        serial.puts("[SMP]   Allocating stack for AP...\n");
        // Allocate stack for AP
        const stack_phys = pmm.allocContiguous(4) orelse {
            serial.puts("[SMP]   Failed to allocate stack for AP\n");
            continue;
        };
        const stack_top = vmm.phys2virt(stack_phys) + 16384;
        serial.puts("[SMP]   Stack allocated. Stack top virtual = 0x");
        serial.putHex(stack_top);
        serial.puts("\n");

        // Pass stack through mailbox
        const stack_mailbox = @as(*volatile u64, @ptrFromInt(vmm.phys2virt(0x7FF8)));
        stack_mailbox.* = stack_top;

        // Init AP CpuLocal
        const ap_cpu = &cpu_local.cpus[cpu_local.cpu_count];
        ap_cpu.cpu_id = @intCast(cpu_local.cpu_count);
        ap_cpu.apic_id = apic_id;
        ap_cpu.self_ptr = @intFromPtr(ap_cpu);

        // Pass CpuLocal pointer through mailbox
        const local_mailbox = @as(*volatile u64, @ptrFromInt(vmm.phys2virt(0x7FE8)));
        local_mailbox.* = @intFromPtr(ap_cpu);
        ap_cpu.kernel_rsp = stack_top;
        ap_cpu.lapic_base = 0xfee00000;

        @atomicStore(u32, &ap_ready_flag, 0, .seq_cst);

        serial.puts("[SMP]   Sending INIT IPI to APIC ");
        serial.putDec(apic_id);
        serial.puts("...\n");
        // Send INIT IPI
        interrupts.lapicWrite(0x310, @as(u32, apic_id) << 24);
        interrupts.lapicWrite(0x300, 0x00004500);

        serial.puts("[SMP]   INIT IPI sent. Waiting...\n");
        // Wait ~10ms (short busy loop; real timing via LAPIC timer in future)
        var delay: u32 = 0;
        while (delay < 10_000) : (delay += 1) asm volatile ("pause");

        serial.puts("[SMP]   Sending SIPI 1...\n");
        // Send SIPI 1
        interrupts.lapicWrite(0x310, @as(u32, apic_id) << 24);
        interrupts.lapicWrite(0x300, 0x00004608); // vector 0x08 = 0x8000 physical

        // Wait ~300us
        delay = 0;
        while (delay < 3_000) : (delay += 1) asm volatile ("pause");

        serial.puts("[SMP]   Sending SIPI 2...\n");
        // Send SIPI 2
        interrupts.lapicWrite(0x310, @as(u32, apic_id) << 24);
        interrupts.lapicWrite(0x300, 0x00004608);

        // Wait ~300us
        delay = 0;
        while (delay < 3_000) : (delay += 1) asm volatile ("pause");

        serial.puts("[SMP]   Polling ap_ready_flag...\n");
        // Poll ap_ready_flag
        var timeout: u32 = 0;
        var ap_started = false;
        while (timeout < 500_000) : (timeout += 1) {
            if (@atomicLoad(u32, &ap_ready_flag, .seq_cst) == 1) {
                ap_started = true;
                break;
            }
            asm volatile ("pause");
        }

        if (ap_started) {
            cpu_local.cpu_count += 1;
            serial.puts("[SMP]   AP CPU ");
            serial.putDec(ap_cpu.cpu_id);
            serial.puts(" online (APIC ID: ");
            serial.putDec(apic_id);
            serial.puts(")\n");
        } else {
            serial.puts("[SMP]   Failed to start AP CPU with APIC ID: ");
            serial.putDec(apic_id);
            serial.puts("\n");
        }
    }
}

pub export fn ap_startup(self: *cpu_local.CpuLocal) callconv(.c) noreturn {
    asm volatile ("cli");
    @import("cpu.zig").enableSse();
    @import("cpu.zig").outb(0x3f8, '1');

    var temp = @intFromPtr(self);
    var i: usize = 0;
    @import("cpu.zig").outb(0x3f8, ':');
    while (i < 16) : (i += 1) {
        const nybble = @as(u8, @truncate((temp >> 60) & 0xF));
        const char = if (nybble < 10) nybble + '0' else nybble - 10 + 'A';
        @import("cpu.zig").outb(0x3f8, char);
        temp <<= 4;
    }
    @import("cpu.zig").outb(0x3f8, ' ');

    var cr3_val: u64 = 0;
    asm volatile ("mov %%cr3, %[out]" : [out] "=r" (cr3_val));
    var temp3 = cr3_val;
    i = 0;
    @import("cpu.zig").outb(0x3f8, 'C');
    @import("cpu.zig").outb(0x3f8, ':');
    while (i < 16) : (i += 1) {
        const nybble = @as(u8, @truncate((temp3 >> 60) & 0xF));
        const char = if (nybble < 10) nybble + '0' else nybble - 10 + 'A';
        @import("cpu.zig").outb(0x3f8, char);
        temp3 <<= 4;
    }
    @import("cpu.zig").outb(0x3f8, ' ');

    // Load AP GDT and TSS
    gdt.init_ap(self.cpu_id, self.kernel_rsp);
    self.tss_ptr = gdt.get_ap_tss(self.cpu_id);
    @import("cpu.zig").outb(0x3f8, '2');

    // Load IDT
    idt.init();
    @import("cpu.zig").outb(0x3f8, '3');

    // Configure GS Base for this AP
    const MSR_GS_BASE: u32 = 0xC0000101;
    const MSR_KERNEL_GS_BASE: u32 = 0xC0000102;
    const lo: u32 = @truncate(@intFromPtr(self) & 0xFFFFFFFF);
    const hi: u32 = @truncate(@intFromPtr(self) >> 32);
    asm volatile ("wrmsr" : : [msr] "{ecx}" (MSR_GS_BASE), [lo] "{eax}" (lo), [hi] "{edx}" (hi));
    asm volatile ("wrmsr" : : [msr] "{ecx}" (MSR_KERNEL_GS_BASE), [lo] "{eax}" (lo), [hi] "{edx}" (hi));
    @import("cpu.zig").outb(0x3f8, '4');

    // Configure Syscall MSRs for this AP
    @import("syscall.zig").init_ap(self);

    // Enable LAPIC
    interrupts.lapicWrite(0xF0, interrupts.lapicRead(0xF0) | 0x100 | 0xFF);
    @import("cpu.zig").outb(0x3f8, '5');

    // Setup LAPIC Timer for AP
    interrupts.setupApTimer();
    @import("cpu.zig").outb(0x3f8, '6');

    // Signal ready back to BSP
    @atomicStore(u32, &ap_ready_flag, 1, .seq_cst);

    // Enable interrupts and idle
    asm volatile ("sti");
    while (true) {
        asm volatile ("hlt");
    }
}
