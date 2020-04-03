# rftp
A remake of `sftp` written in Rust.

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