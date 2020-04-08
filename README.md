# rftp
A remake of `sftp` written in Rust.

[![Crates.io](https://img.shields.io/crates/v/rftp)](https://crates.io/crates/rftp)
[![Build Status](https://travis-ci.org/ellishg/rftp.svg?branch=master)](https://travis-ci.org/ellishg/rftp)

## Installation
This will install `rftp` to `~/.cargo/bin`.
```bash
cargo install --path .
```

## Usage
```bash
rftp <destination> -u <username> -p <port>
```

Use the arrow keys to navigate the local and remote files, the enter key to enter directories, the spacebar to download/upload the file, and 'q' to quit.