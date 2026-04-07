# CLAUDE.md

Read `AGENTS.md` first for architectural context and module map.

## Build

```bash
cargo build                                        # Debug (default: native)
cargo build --release                              # Release
cargo build --features ios                         # iOS (no audio output)
cargo run --example kick --features native,crossterm  # Run examples
```

## Test

```bash
cargo test --verbose                               # All tests
cargo test test_engine_creation --verbose           # Single test
cargo test --test engine_basics --verbose           # Single test file
cargo test modulation --verbose                     # Pattern match
```

## Validate

After any code changes, run all of these before considering the work done:

```bash
cargo build                                        # Library builds
cargo build --example kick --features native,crossterm  # Examples typecheck
cargo test --verbose                               # All tests pass
cargo fmt --all -- --check                         # Formatting
cargo clippy --all-targets --all-features          # Clippy
```

## Lint

```bash
cargo fmt --all -- --check                         # Check formatting
cargo fmt --all                                    # Apply formatting
cargo clippy --all-targets --all-features          # Clippy
```

## Code Style

- **Imports**: std → external crates → `crate::`/`super::`, separated by blank lines
- **Types**: `f32` for audio values, `f64` for time accumulation
- **Naming**: PascalCase structs, snake_case functions, SCREAMING_SNAKE constants
- **Errors**: `Result<T, String>` in library code, `anyhow::Result` in examples
- **Params**: Validate with `.clamp()` in constructors

## FFI (`src/ffi.rs`)

- `#[no_mangle] extern "C"` functions with null pointer checks
- `Box::into_raw` / `Box::from_raw` for heap allocation
- `/// # Safety` doc section on all unsafe functions
- `build.rs` runs cbindgen to generate `include/gooey.h`

## Conditional Compilation

```rust
#[cfg(feature = "native")]
pub mod engine_output;
```
