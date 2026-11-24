use fltk::{
    button::Button,
    draw::*,
    enums::{Align, Color, Font, FrameType},
    frame::Frame,
    prelude::*,
};

pub const PAPER: Color = Color::from_hex(0xf6f4ef);
pub const CARD: Color = Color::from_hex(0xfffefa);
pub const INK: Color = Color::from_hex(0x252924);
pub const MUTED: Color = Color::from_hex(0x62665e);
pub const BORDER: Color = Color::from_hex(0xe3dfd5);
pub const RUST: Color = Color::from_hex(0xa94026);
pub const RUST_HOVER: Color = Color::from_hex(0x90331e);
pub const TINT: Color = Color::from_hex(0xf6e6dc);
pub const DARK: Color = Color::from_hex(0x222621);
pub const DARK_TOP: Color = Color::from_hex(0x2b3029);
pub const DARK_EDGE: Color = Color::from_hex(0x40463d);
pub const LIGHT: Color = Color::from_hex(0xeeeee6);
pub const DIM_LIGHT: Color = Color::from_hex(0xaeb7a8);
pub const GREEN: Color = Color::from_hex(0x43704d);

pub fn rounded(x: i32, y: i32, w: i32, h: i32, radius: i32, fill: Color, edge: Color) {
    set_draw_color(fill);
    draw_rounded_rectf(x, y, w, h, radius);
    set_draw_color(edge);
    draw_rounded_rect(x, y, w, h, radius);
}

pub fn panel(fill: Color, edge: Color) -> Frame {
    let mut frame = Frame::default();
    frame.draw(move |f| rounded(f.x(), f.y(), f.w() - 1, f.h() - 1, 12, fill, edge));
    frame
}

pub fn label(text: &str, font: Font, size: i32, color: Color) -> Frame {
    let mut frame = Frame::default().with_label(text);
    frame.set_label_font(font);
    frame.set_label_size(size);
    frame.set_label_color(color);
    frame.set_align(Align::Inside | Align::Left | Align::Clip);
    frame
}

pub fn line(x: i32, y: i32, end_x: i32, end_y: i32, color: Color) {
    set_draw_color(color);
    draw_line(x, y, end_x, end_y);
}

pub fn icon(name: &str, x: i32, y: i32, color: Color) {
    set_draw_color(color);
    set_line_style(LineStyle::Solid, 1);
    let l = |a, b, c, d| draw_line(x + a, y + b, x + c, y + d);
    let rect = |a, b, w, h| draw_rect(x + a, y + b, w, h);
    match name {
        "file" => {
            rect(3, 1, 10, 14);
            l(5, 6, 10, 6);
            l(5, 9, 10, 9);
            l(5, 12, 8, 12);
        }
        "calc" => {
            rect(2, 1, 12, 14);
            rect(5, 4, 6, 3);
            l(5, 10, 6, 10);
            l(9, 10, 10, 10);
            l(5, 12, 6, 12);
            l(9, 12, 10, 12);
        }
        "graph" => {
            l(1, 14, 15, 14);
            l(4, 12, 4, 8);
            l(8, 12, 8, 3);
            l(12, 12, 12, 6);
        }
        "disk" => {
            rect(1, 4, 14, 9);
            l(2, 9, 14, 9);
            l(4, 11, 5, 11);
        }
        "net" => {
            rect(6, 1, 5, 4);
            rect(1, 11, 5, 4);
            rect(11, 11, 5, 4);
            l(8, 5, 8, 8);
            l(3, 8, 13, 8);
            l(3, 8, 3, 11);
            l(13, 8, 13, 11);
        }
        "search" => {
            draw_arc(x + 1, y + 1, 10, 10, 0.0, 360.0);
            l(10, 10, 14, 14);
        }
        "folder" => {
            l(1, 4, 6, 4);
            l(6, 4, 8, 6);
            l(8, 6, 15, 6);
            l(1, 4, 1, 13);
            l(1, 13, 14, 13);
            l(14, 13, 15, 6);
        }
        "lock" => {
            draw_arc(x + 4, y + 1, 8, 8, 0.0, 180.0);
            l(4, 5, 4, 7);
            l(11, 5, 11, 7);
            rect(2, 7, 12, 8);
            l(8, 10, 8, 12);
        }
        "window" => {
            rect(1, 2, 14, 12);
            l(1, 5, 14, 5);
        }
        "open" => {
            l(8, 2, 14, 2);
            l(14, 2, 14, 8);
            l(14, 2, 7, 9);
            l(6, 4, 2, 4);
            l(2, 4, 2, 14);
            l(2, 14, 12, 14);
            l(12, 14, 12, 10);
        }
        "play" => {
            draw_polygon(x + 4, y + 2, x + 4, y + 14, x + 13, y + 8);
        }
        "stop" => {
            draw_rectf(x + 4, y + 4, 8, 8);
        }
        _ => {
            l(2, 4, 6, 8);
            l(6, 8, 2, 12);
            l(8, 12, 14, 12);
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ButtonStyle {
    Primary,
    Neutral,
    Dark,
    Tab,
}

pub fn button(text: &str, glyph: &'static str, style: ButtonStyle) -> Button {
    let mut button = Button::default().with_label(text);
    button.set_frame(FrameType::NoBox);
    button.draw(move |b| {
        let (x, y, w, h) = (b.x(), b.y(), b.w() - 1, b.h() - 1);
        let pressed = b.value();
        let active = b.active();
        let hovered = app::belowmouse::<fltk::widget::Widget>()
            .is_some_and(|widget| widget.as_widget_ptr() == b.as_widget_ptr());
        let (base, edge, text) = match style {
            ButtonStyle::Primary if active => {
                let color = if hovered || pressed { RUST_HOVER } else { RUST };
                (color, color, Color::White)
            }
            ButtonStyle::Primary => (TINT, TINT, MUTED),
            ButtonStyle::Neutral => (
                if hovered || pressed { PAPER } else { CARD },
                BORDER,
                if active { INK } else { MUTED },
            ),
            ButtonStyle::Dark => (
                if hovered || pressed {
                    DARK_EDGE
                } else {
                    DARK_TOP
                },
                DARK_EDGE,
                if active { LIGHT } else { DIM_LIGHT },
            ),
            ButtonStyle::Tab if pressed => (CARD, BORDER, INK),
            ButtonStyle::Tab => (PAPER, PAPER, MUTED),
        };
        rounded(x, y, w, h, 7, base, edge);
        if b.has_focus() {
            set_draw_color(if style == ButtonStyle::Dark {
                DIM_LIGHT
            } else {
                RUST
            });
            draw_rounded_rect(x + 1, y + 1, w - 2, h - 2, 6);
        }
        let caption = b.label();
        set_font(Font::HelveticaBold, 13);
        let text_width = width(&caption).ceil() as i32;
        let content_width = text_width + if glyph.is_empty() { 0 } else { 22 };
        let mut left = x + (w - content_width) / 2;
        if !glyph.is_empty() {
            icon(glyph, left, y + (h - 16) / 2, text);
            left += 22;
        }
        set_draw_color(text);
        draw_text2(
            &caption,
            left,
            y,
            text_width + 4,
            h,
            Align::Left | Align::Inside,
        );
    });
    button.handle(|b, event| {
        if matches!(event, fltk::enums::Event::Enter | fltk::enums::Event::Leave) {
            b.redraw();
        }
        false
    });
    button
}

use fltk::app;
