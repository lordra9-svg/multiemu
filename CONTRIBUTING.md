# Contributing

Thanks for taking a look at multiemu. The project is MIT-licensed and aims to be
a clean, multi-system emulator shell with a reference-parity NES core.

## Ground rules

- **No ROMs, BIOS images, or copyrighted firmware** may be committed. Tests
  that need public test ROMs look under `test-roms/` and skip when it is
  absent.
- **No GPL source** (MesenCE, RustyNES, Dolphin, gopher64) may be copied,
  translated, vendored or linked. They are behaviour oracles only.
- ares (ISC) may be adapted with attribution. Record every such adaptation in
  `crates/multiemu-nes-accuracy/PROVENANCE.md` and keep the ares notice in
  `NOTICE` / `licenses/ares-LICENSE.txt`.
- Keep the accuracy claims honest. Do not raise the accuracy badge, and do not
  remove a divergence from `docs/accuracy-divergences.md`, without a passing
  test or a hardware capture.

## Build and check

```sh
cargo build --release
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

`cargo test` and the AccuracyCoin example skip cleanly when `test-roms/` is not
present.

## Accuracy workflow

1. Pick a failure from `docs/accuracy-divergences.md`.
2. Reproduce it with the isolated runner:

   ```sh
   cargo run -p multiemu-nes-accuracy --release --example accuracycoin -- \
     test-roms/AccuracyCoin.nes test-roms/AccuracyCoin.tsv --isolated --only 46A
   ```

3. Make the smallest change that fixes the sub-test. Prefer matching the
   documented behaviour over answer-key patching.
4. Re-run the isolated test, the nearby tests in the same suite, and the full
   battery:

   ```sh
   cargo run -p multiemu-nes-accuracy --release --example accuracycoin -- \
     test-roms/AccuracyCoin.nes test-roms/AccuracyCoin.tsv --isolated
   ```

5. Update `docs/accuracy-status.md` (dated log) and
   `docs/accuracy-divergences.md` (open list / recently closed).

## Pull requests

- One logical change per PR, with the test evidence in the description.
- `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D
  warnings` and `cargo test --workspace` must pass.
- Call out any behaviour change that is not covered by a test ROM.
