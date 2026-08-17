# Vanta service runtime contract

`vanta-services` is the bounded request/response contract shared by native
services and the kernel service supervisor. It defines fixed-size 256-byte wire
frames, service identity/discovery, request IDs, generation-bearing capability
authority, restart and crash accounting, capability revocation, and a bounded
audit ring.

The no-std supervisor remains a host-testable policy reference. The kernel now
also exposes bounded IPC channel descriptors, and the GPT image boots a real
`procd`, a crashing predecessor, and an upgraded `/bin/vfsd` backend. The
current acceptance proves bounded blocking request/response, restart after a
nonzero service exit, framed registration/discovery, a VFS read through the
service, filesystem-backed audit persistence across reboot, and authority
revocation without changing the frozen native ABI. The QEMU service flow now
exchanges these framed records rather than ad-hoc command strings.

```powershell
cargo test -p vanta-services
```
