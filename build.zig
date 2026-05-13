const std = @import("std");

pub fn build(b: *std.Build) void {
    // ── Target: Freestanding x86_64 ──────────────────────────────
    var target_query: std.Target.Query = .{
        .cpu_arch = .x86_64,
        .os_tag = .freestanding,
        .abi = .none,
    };

    // NOTE: We keep SSE/SSE2 enabled — x86_64 mandates them and the Zig
    // std lib (ubsan, Writer) uses f128 which needs SSE. We'll handle FPU
    // state save/restore in the scheduler (Phase 1) with lazy FPU switching.
    // Only disable AVX (optional, wider registers = more context save cost).
    target_query.cpu_features_sub = std.Target.x86.featureSet(&.{
        .avx,
        .avx2,
    });

    const target = b.resolveTargetQuery(target_query);
    const optimize = b.standardOptimizeOption(.{});

    // ── Kernel executable ────────────────────────────────────────
    // Zig 0.16: addExecutable takes root_module: *Module via createModule
    const kernel = b.addExecutable(.{
        .name = "vanta",
        .use_llvm = true, // Self-hosted x86 backend has limited inline asm; use LLVM
        .use_lld = true,
        .root_module = b.createModule(.{
            .root_source_file = b.path("kernel/main.zig"),
            .target = target,
            .optimize = optimize,
            .red_zone = false, // Red zone is unsafe in interrupt handlers
            .stack_check = false, // No stack probes in freestanding
            .code_model = .kernel, // Higher-half kernel needs kernel code model
        }),
    });

    kernel.setLinkerScript(b.path("linker.ld"));

    // Install the kernel ELF to zig-out/bin/vanta
    b.installArtifact(kernel);

    // ── Convenience step: `zig build run` ────────────────────────
    // Builds kernel, creates ISO, runs QEMU (requires xorriso + qemu)
    const run_cmd = b.addSystemCommand(&.{
        "bash", "scripts/run-qemu.sh",
    });
    run_cmd.step.dependOn(b.getInstallStep());

    const run_step = b.step("run", "Build and run VantaOS in QEMU");
    run_step.dependOn(&run_cmd.step);
}
