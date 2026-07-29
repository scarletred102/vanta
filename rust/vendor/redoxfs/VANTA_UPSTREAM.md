# Vanta RedoxFS provenance

This directory is a squashed subtree import of RedoxFS from
`https://gitlab.redox-os.org/redox-os/redoxfs`.

- Upstream commit: `99bc185bf8ad8bd6f4d2562c424d800c2a3d310b`
- Upstream package version: `0.9.1`
- License: MIT; see `LICENSE`
- Import command: `git subtree add --prefix=rust/vendor/redoxfs <upstream> 99bc185bf8ad8bd6f4d2562c424d800c2a3d310b --squash`

Vanta consumes the filesystem core with default features disabled. Vanta-owned
adapters live outside this subtree so that upstream changes and local changes
remain separately reviewable.

Vanta patches the formatter path so unencrypted `FileSystem::create` is
available without RedoxFS's host-only `std` feature. In that profile encrypted
formatting returns `EOPNOTSUPP`, and new headers use a zero UUID; the host
image builder creates only unencrypted filesystems. The upstream `std` profile
retains random UUID and encrypted-format support.

Vanta also adds `Transaction::create_node_with_owner` while preserving the
original parent-owned `create_node` API. Vanta's adapter uses this narrow patch
to provision explicit UID/GID metadata for bootstrap directories such as
`/home/vanta`, and for future credential-aware creates. Preserve this patch
when updating the subtree and review it against the corresponding upstream
transaction implementation.

To update, fetch the intended upstream commit, review its license and diff,
then run `git subtree pull --prefix=rust/vendor/redoxfs <upstream> <commit> --squash`.
