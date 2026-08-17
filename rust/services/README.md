# Vanta service runtime contract

`vanta-services` is the bounded request/response contract shared by native
services and the kernel service supervisor. It defines fixed-size IPC frames,
service lifecycle states, capability authority on every request, restart and
crash accounting, capability revocation, and a bounded audit ring.

The no-std supervisor remains a host-testable policy reference. The kernel now
also exposes bounded IPC channel descriptors, and the GPT image boots a real
`procd` plus a crash/restart service. The current acceptance proves bounded
blocking request/response, restart after a nonzero service exit, upgrade,
filesystem-backed audit persistence across reboot, and authority revocation
without changing the frozen native ABI.

```powershell
cargo test -p vanta-services
```
