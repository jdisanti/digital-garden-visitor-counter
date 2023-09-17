# Digital Garden Visitor Counter

## Required tools for building

The following are needed to build and deploy this Lambda:
- [Rust](https://rustup.rs/)
- [NodeJS (18.x or later)](https://nodejs.org/)
- [Zig](https://ziglang.org/)
- [Just](https://crates.io/crates/just) (`cargo install --locked just` after installing Rust)
- [Cargo Lambda](https://www.cargo-lambda.info/guide/installation.html)

## Building

The Lambda can be tested and built with:
```
just test
just synth
```

## Deploying

