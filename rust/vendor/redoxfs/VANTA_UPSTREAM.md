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

To update, fetch the intended upstream commit, review its license and diff,
then run `git subtree pull --prefix=rust/vendor/redoxfs <upstream> <commit> --squash`.
