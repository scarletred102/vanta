# Architectural Decision: Limine GDT Exclusivity and IDT

## Context
During phase 0/early boot, VantaOS defers initializing its own GDT to Phase 1, choosing instead to reuse the GDT implicitly provided by the Limine bootloader.
When the kernel reaches `idt.init()`, it attempts to load a basic IDT to catch early exceptions.

## Issue
The boot sequence crashed (triple-faulted, leading to QEMU exit) immediately after announcing intent to use Limine's GDT, prior to announcing that the IDT was loaded. 

This occurred due to `kernel/arch/x86_64/idt.zig` setting `.selector = 0x08` for its interrupt gates.
In the Limine bootloader's GDT environment, `0x08` corresponds to the 16-bit code segment descriptor. The correct 64-bit kernel code segment descriptor required for interrupt vectors is `0x28`. 

Thus, upon loading the IDT (`lidt`), any subsequent exception (such as one stemming from uninitialized memory usage, or any potential unmaskable exceptions under testing) vectors through an invalid 16-bit descriptor, resulting in a `#GP` -> `#DF` -> Triple Fault loop. 
Additionally, the stack allocation layout for the IDTR pointer (`var idtr: [10]u8 align(4) = undefined;`) passed to inline assembly via the array mapping `"m"` runs the risk of generating instructions referencing unaligned base pointers that can further perturb standard `lidt` instruction execution boundaries, depending on zig compiler layout decisions.

## Resolution
1. **Selector Target (`idt.zig`)**: The `makeGate(handler_addr: u64)` selector property has been switched to `.selector = 0x28` to maintain correct correspondence identically mapped to Limine's 64-bit Code Segment selector.
2. **IDTR Pointer Strictness**: The `idtr` object structure has been refactored into a `packed struct` encompassing the `limit: u16` and `base: u64` fields appropriately, ensuring aligned parameter loading cleanly passing via pointer constraints (`[ptr] "*p" (&idtr)`).