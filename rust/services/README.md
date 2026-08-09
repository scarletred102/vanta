# Vanta service runtime contract

`vanta-services` is the bounded request/response contract shared by native
services and the kernel service supervisor. It defines fixed-size IPC frames,
service lifecycle states, capability authority on every request, restart and
crash accounting, capability revocation, and a bounded audit ring.

The supervisor is a no-std, host-testable reference implementation. The next
integration step is to transport `IpcFrame` through kernel channels and launch
the first `vfsd`/`procd` service processes without changing the frozen native
ABI.

```powershell
cargo test -p vanta-services
```
