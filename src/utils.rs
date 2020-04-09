use tui::{buffer::Buffer, style::Style};

#[allow(dead_code)]
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
