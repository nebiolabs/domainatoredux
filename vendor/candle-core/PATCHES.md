# domainatoredux local patch (not upstream)

This directory is the published `candle-core` 0.11.1 crate (crates.io), wired in through
`[patch.crates-io]` in the workspace `Cargo.toml`. One line differs from the release:

- `Cargo.toml`, the `cudarc` dependency's features: `"dynamic-linking"` →
  `"dynamic-loading"`.

With `dynamic-linking`, cudarc's build script links the driver's `libcuda`, cuBLAS,
cuBLASLt, cuRAND and NVRTC into the binary, so a `cuda` build does not start on a machine
without them. With `dynamic-loading`, cudarc opens each library with `libloading` the first
time it is used. domainatoredux checks each one with cudarc's `is_culib_present()` before
opening a device (`domainator-plm`'s `select_device`), because cudarc panics on a missing
library. Without them, the build falls back to the CPU.

Check the result with `diff -r ~/.cargo/registry/src/*/candle-core-0.11.1 vendor/candle-core`.
To drop the patch, see UPSTREAM.md, "Patched dependency: `candle-core`".
