# rftp
A remake of `sftp` written in Rust.

[![Crates.io](https://img.shields.io/crates/v/rftp)](https://crates.io/crates/rftp)
[![Build Status](https://github.com/ellishg/rftp/actions/workflows/rust.yml/badge.svg)](https://github.com/ellishg/rftp/actions/workflows/rust.yml)

![A demo of rftp](assets/demo.gif)

## Installation
This will install `rftp` to `~/.cargo/bin`.
```bash
cargo install rftp
```

## Usage
```bash
rftp <destination> -u <username> -p <port>
```

## Controls

| Key | Function |
|:---|:--------|
| Arrow keys<br>**h**/**j**/**k**/**l** | Navigate the files                |
| Enter      | Enter into the selected directory |
| Spacebar   | Download/Upload the selected file |
| **z**      | Show/hide hidden files            |
| **q**      | Quit                              |
| **Q**      | Force quit                        |
| **?**      | Print help message                |

## TODO

- [ ] Create new directories
- [x] Upload/download directories recursively
- [x] Show/hide invisible files
