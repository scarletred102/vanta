// ============================================================================
// VantaOS -- x86_64 CPU feature setup
// ============================================================================

pub inline fn enableSse() void {
    var cr0 = asm volatile ("mov %%cr0, %[ret]"
        : [ret] "=r" (-> u64),
    );

    // Enable monitor coprocessor and clear emulation so SSE instructions work.
    cr0 |= @as(u64, 1) << 1; // MP
    cr0 &= ~(@as(u64, 1) << 2); // EM

    asm volatile ("mov %[val], %%cr0"
        :
        : [val] "r" (cr0),
        : .{ .memory = true }
    );

    var cr4 = asm volatile ("mov %%cr4, %[ret]"
        : [ret] "=r" (-> u64),
    );

    cr4 |= (@as(u64, 1) << 9) | (@as(u64, 1) << 10); // OSFXSR | OSXMMEXCPT

    asm volatile ("mov %[val], %%cr4"
        :
        : [val] "r" (cr4),
        : .{ .memory = true }
    );
}

pub inline fn outb(port: u16, val: u8) void {
    asm volatile ("outb %[val], %[port]"
        :
        : [val] "{al}" (val),
          [port] "{dx}" (port),
    );
}

pub inline fn inb(port: u16) u8 {
    return asm volatile ("inb %[port], %[ret]"
        : [ret] "={al}" (-> u8),
        : [port] "{dx}" (port),
    );
}

pub inline fn outl(port: u16, val: u32) void {
    asm volatile ("outl %[val], %[port]"
        :
        : [val] "{eax}" (val),
          [port] "{dx}" (port),
    );
}

pub inline fn inl(port: u16) u32 {
    return asm volatile ("inl %[port], %[ret]"
        : [ret] "={eax}" (-> u32),
        : [port] "{dx}" (port),
    );
}

pub inline fn rdmsr(msr: u32) u64 {
    var low: u32 = 0;
    var high: u32 = 0;
    asm volatile ("rdmsr"
        : [low] "={eax}" (low),
          [high] "={edx}" (high),
        : [msr] "{ecx}" (msr),
    );
    return (@as(u64, high) << 32) | low;
}

pub inline fn wrmsr(msr: u32, val: u64) void {
    const low: u32 = @truncate(val);
    const high: u32 = @truncate(val >> 32);
    asm volatile ("wrmsr"
        :
        : [msr] "{ecx}" (msr),
          [low] "{eax}" (low),
          [high] "{edx}" (high),
        : .{ .memory = true }
    );
}

pub inline fn cli() void {
    asm volatile ("cli" ::: "memory");
}

pub inline fn sti() void {
    asm volatile ("sti" ::: "memory");
}

