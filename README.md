# rftp
A remake of `sftp` written in Rust.

[![Crates.io](https://img.shields.io/crates/v/rftp)](https://crates.io/crates/rftp)
[![Build Status](https://travis-ci.org/ellishg/rftp.svg?branch=master)](https://travis-ci.org/ellishg/rftp)

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
| **q**      | Quit                              |
| **Q**      | Force quit                        |
