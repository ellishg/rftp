use std::time::Duration;
use tui::{buffer::Buffer, style::Style};

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

#[allow(dead_code)]
/// Return a `Buffer` that is the same, but with the default style.
pub fn buffer_without_style(buffer: &Buffer) -> Buffer {
    let mut buffer = buffer.clone();
    let rect = buffer.area().clone();
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
}
