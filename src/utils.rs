
use std::io::Read;
use std::path::PathBuf;
use std::time::Duration;
use thiserror::Error;
use tui::{buffer::Buffer, style::Style};

pub type Result<T> = std::result::Result<T, ErrorKind>;

#[derive(Error, Debug)]
pub enum ErrorKind {
    #[error("unable to authenticate session for {0}")]
    UserAuthenticationError(String),

    #[error("the authenticity of {0}:{1} cannot be established")]
    HostAuthenticationError(String, u16),

    #[error("unable to check known hosts")]
    HostFileCheckError,

    #[error("unable to parse port number")]
    InvalidPortNumber,

    #[error("possible person in the middle attack")]
    MismatchedFingerprint,

    #[error("unable to find home directory")]
    UnableToFindHomeDirectory,

    #[error("unable to find host key from session")]
    HostKeyNotFound,

    #[error("unable to find fingerprint of host")]
    HostFingerprintNotFound,

    #[error("cannot download parent directory {0}")]
    CannotDownloadParent(String),

    #[error("cannot upload parent directory {0}")]
    CannotUploadParent(String),

    #[error("local file {0} already exists")]
    LocalFileExists(String),

    #[error("remote file {0} already exists")]
    RemoteFileExists(String),

    #[error("channel closed with exit status {0}")]
    Ssh2ChannelClosed(i32),

    #[error(transparent)]
    Ssh2Error(#[from] ssh2::Error),

    #[error(transparent)]
    CrosstermError(#[from] crossterm::ErrorKind),

    #[error(transparent)]
    IOError(#[from] std::io::Error),

    #[error(transparent)]
    VarError(#[from] std::env::VarError),
}

/// Return the path to the host home directory.
pub fn get_remote_home_dir(session: &ssh2::Session) -> Result<PathBuf> {
    let mut channel = session.channel_session()?;
    channel.exec("pwd")?;
    let mut result = String::new();
    channel.read_to_string(&mut result)?;
    let result = result.trim();
    channel.wait_close()?;
    let exit_status = channel.exit_status()?;
    if exit_status == 0 {
        Ok(PathBuf::from(result))
    } else {
        Err(ErrorKind::Ssh2ChannelClosed(exit_status))
    }
}

/// Returns a `String` that represents a `Duration` in hours, minutes, and seconds.
pub fn duration_to_string(t: Duration) -> String {
    let seconds = t.as_secs();
    let (seconds, minutes) = (seconds % 60, seconds / 60);
    let (minutes, hours) = (minutes % 60, minutes / 60);
    if hours > 0 {
        format!("{}:{:02}:{:02}", hours, minutes, seconds)
    } else {
        format!("{:02}:{:02}", minutes, seconds)
    }
}

/// Returns a `String` that represents a bitrate.
pub fn bitrate_to_string(rate: u64) -> String {
    if rate < 1_000 {
        format!("{} bit/s", rate)
    } else if rate < 1_000_000 {
        format!("{:.1} Kbit/s", rate as f64 / 1e3)
    } else if rate < 1_000_000_000 {
        format!("{:.1} Mbit/s", rate as f64 / 1e6)
    } else {
        format!("{:.1} Gbit/s", rate as f64 / 1e9)
    }
}

pub fn bytes_to_string(num_bytes: u64) -> String {
    if num_bytes < 1_000 {
        format!("{} B", num_bytes)
    } else if num_bytes < 1_000_000 {
        format!("{:.1} KB", num_bytes as f64 / 1e3)
    } else if num_bytes < 1_000_000_000 {
        format!("{:.1} MB", num_bytes as f64 / 1e6)
    } else {
        format!("{:.1} GB", num_bytes as f64 / 1e9)
    }
}

#[allow(dead_code)]
/// Return a `Buffer` that is the same, but with the default style.
pub fn buffer_without_style(buffer: &Buffer) -> Buffer {
    let mut buffer = buffer.clone();
    let rect = *buffer.area();
    for x in rect.x..rect.width {
        for y in rect.y..rect.height {
            buffer.get_mut(x, y).set_style(Style::default());
        }
    }
    buffer
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_duration() {
        assert_eq!(
            duration_to_string(Duration::from_secs(12)),
            "00:12".to_string()
        );
        assert_eq!(
            duration_to_string(Duration::from_secs(123)),
            "02:03".to_string()
        );
        assert_eq!(
            duration_to_string(Duration::from_secs(2 * 60 * 60 + 40 * 60 + 59)),
            "2:40:59".to_string()
        );
    }

    #[test]
    fn test_bitrate() {
        assert_eq!(bitrate_to_string(4), "4 bit/s".to_string());
        assert_eq!(bitrate_to_string(1e3 as u64), "1.0 Kbit/s".to_string());
        assert_eq!(bitrate_to_string(3e6 as u64), "3.0 Mbit/s".to_string());
        assert_eq!(bitrate_to_string(7e9 as u64), "7.0 Gbit/s".to_string());
    }

    #[test]
    fn test_bytes() {
        assert_eq!(bytes_to_string(849), "849 B".to_string());
        assert_eq!(bytes_to_string(3e3 as u64), "3.0 KB".to_string());
        assert_eq!(bytes_to_string(6e6 as u64), "6.0 MB".to_string());
        assert_eq!(bytes_to_string(2e9 as u64), "2.0 GB".to_string());
    }
}
