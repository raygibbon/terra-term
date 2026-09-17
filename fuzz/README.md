# Rust-only parser fuzz entry point

`cargo run --manifest-path fuzz/Cargo.toml --bin parser < input.bin` feeds up to 1 MiB of arbitrary bytes through the incremental input parser at several fragment sizes. The process exits nonzero on a panic. It uses no C fuzzing runtime.

`cargo run --manifest-path fuzz/Cargo.toml --bin terminfo < entry.bin` feeds up to 1 MiB into the compiled terminfo parser and cursor capability evaluator. The harness compiles those private source modules inside its separate crate, so no fuzz-only function appears in terra-term's public API. Malformed data also has a deterministic property test in `src/terminfo/mod.rs`.

`cargo run --manifest-path fuzz/Cargo.toml --bin capability < capability.bin` feeds arbitrary parameter templates (after four bytes used as row and column inputs) into the bounded evaluator.
