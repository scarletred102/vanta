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
