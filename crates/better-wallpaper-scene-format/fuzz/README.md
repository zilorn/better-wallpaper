# Scene format fuzzing

Install nightly Rust and `cargo-fuzz`, then run each parser target:

```sh
rustup toolchain install nightly
cargo install cargo-fuzz
cd crates/better-wallpaper-scene-format
cargo +nightly fuzz run pkg_header -- -max_len=1048576
cargo +nightly fuzz run pkg_index -- -max_len=1048576
cargo +nightly fuzz run pkg_string -- -max_len=4096
cargo +nightly fuzz run texture -- -max_len=1048576
cargo +nightly fuzz run scene -- -max_len=1048576
```

Run targets independently so crashes and resource usage remain attributable to a
single parser. Corpus and artifact directories are intentionally not committed.
