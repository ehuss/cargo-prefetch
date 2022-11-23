# `cargo prefetch`

[![crates.io](https://img.shields.io/crates/v/cargo-prefetch.svg)](https://crates.io/crates/cargo-prefetch)

A [Cargo] subcommand to download popular crates.

This command is used to download some popular dependencies into Cargo's cache.
This is useful if you plan to go offline, and you want a collection of common
crates available to use.

[Cargo]: https://doc.rust-lang.org/cargo/

## Installation

`cargo install cargo-prefetch`

## Usage

Running `cargo prefetch` will download the top 100 most common dependencies on
[crates.io]. There are several options for choosing which crates will be
downloaded, run with `--help` to see the options.

[crates.io]: https://crates.io/

### Examples

1. `cargo prefetch`

    Downloads the top 100 most common dependencies.

2. `cargo prefetch --list`

    Print what would be downloaded, instead of downloading.

3. `cargo prefetch serde`

    Downloads the most recent version of [serde].

4. `cargo prefetch serde@=1.0.90`

    Download a specific version of serde.

5. `cargo prefetch --top-downloads`

    Download the top 100 most downloaded crates.

6. `cargo prefetch --top-downloads=400`

    Download the top 400 most downloaded crates.
 
8. `cargo prefetch --lockfile Cargo.lock`

    Download the crates listed in `Cargo.lock` (the flattened transitive dependency graph of a project).

[serde]: https://crates.io/crates/serde
