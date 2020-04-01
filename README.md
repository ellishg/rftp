# rftp
A remake of `sftp` written in Rust.

## Usage
```bash
rftp <destination> -u <username> -p <port>
```

Use the arrow keys to navigate the local and remote files, the enter key to enter directories, the spacebar to download/upload the file, and the escape key to quit. **Due to a bug, you need to press the enter key after pressing the escape key to quit.**