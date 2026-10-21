# Dependency exclusion rejection test

Use a disposable checkout containing this package. The production checkout is
not modified. From its `firmware-rust` directory:

1. Add this normal dependency under `[dependencies]` in
   `apps/m1-firmware/Cargo.toml`:

   ```toml
   pajoniiir-apta-adapter = { path = "../../crates/pajoniiir-apta-adapter" }
   ```

2. Resolve the disposable firmware lock so the test reaches graph inspection:

   ```sh
   cargo tree --manifest-path apps/m1-firmware/Cargo.toml --target riscv32imafc-unknown-none-elf --edges normal,build --prefix none
   ```

3. Run the same CI exclusion command:

   ```sh
   python3 tools/check_apta_core_boundary.py
   ```

Expected result: a nonzero exit with
`firmware default: experimental provider dependency: ['libapta', 'pajoniiir-apta-adapter']`.
The preceding desktop graph must still pass. A Cargo resolution failure is not
the expected successful rejection: it must reach the explicit dependency error.

The retained `apta-core-negative.log` records that result. The diagnostic clone
was based on main `e069888ccce3ef23412741607b54058c73d9e82f` with the exact
reviewed selective patch applied before dependency injection. Its patch hash is
recorded in `manifest.json`. No injected change is part of the committed package.
