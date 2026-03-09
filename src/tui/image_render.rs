use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};

/// Render image to colored halfblock lines.
pub(super) fn render_image_to_colored_lines(
    img: &image::DynamicImage,
    max_width: u32,
    max_height: u32,
) -> Vec<Line<'static>> {
    use image::GenericImageView;

    let (orig_w, orig_h) = img.dimensions();
    let orig_aspect = orig_w as f32 / orig_h as f32;

    let target_width = max_width;
    let target_height_chars = ((target_width as f32 / orig_aspect) / 2.0) as u32;

    let (final_width, final_height_chars) = if target_height_chars > max_height {
        let h = max_height;
        let w = (h as f32 * 2.0 * orig_aspect) as u32;
        (w, h)
    } else {
        (target_width, target_height_chars)
    };

    let img = img.resize(
        final_width,
        final_height_chars * 2,
        image::imageops::FilterType::Lanczos3,
    );

    let (w, h) = img.dimensions();
    let mut lines = Vec::new();

    for y in 0..final_height_chars {
        let mut spans = Vec::new();
        for x in 0..w {
            let y_top = (y * 2).min(h - 1);
            let y_bottom = (y * 2 + 1).min(h - 1);

            let top_pixel = img.get_pixel(x, y_top);
            let bottom_pixel = img.get_pixel(x, y_bottom);

            let span = Span::styled(
                "\u{2580}",
                Style::default()
                    .fg(Color::Rgb(top_pixel[0], top_pixel[1], top_pixel[2]))
                    .bg(Color::Rgb(bottom_pixel[0], bottom_pixel[1], bottom_pixel[2])),
            );
            spans.push(span);
        }
        lines.push(Line::from(spans));
    }

    lines
}

/// Render image to grayscale ASCII art lines.
pub(super) fn render_image_to_grayscale_lines(
    img: &image::DynamicImage,
    max_width: u32,
    max_height: u32,
) -> Vec<Line<'static>> {
    use image::GenericImageView;

    const ASCII_CHARS: &[char] = &[' ', '.', ':', '-', '=', '+', '*', '#', '%', '@'];

    let (orig_w, orig_h) = img.dimensions();
    let orig_aspect = orig_w as f32 / orig_h as f32;

    let target_width = max_width;
    let target_height_chars = ((target_width as f32 / orig_aspect) / 2.0) as u32;

    let (final_width, final_height_chars) = if target_height_chars > max_height {
        let h = max_height;
        let w = (h as f32 * 2.0 * orig_aspect) as u32;
        (w, h)
    } else {
        (target_width, target_height_chars)
    };

    let img = img.resize(
        final_width,
        final_height_chars * 2,
        image::imageops::FilterType::Lanczos3,
    );

    let (w, h) = img.dimensions();
    let mut lines = Vec::new();

    for y in 0..final_height_chars {
        let mut line_str = String::new();
        for x in 0..w {
            let y1 = (y * 2).min(h - 1);
            let y2 = (y * 2 + 1).min(h - 1);

            let pixel1 = img.get_pixel(x, y1);
            let pixel2 = img.get_pixel(x, y2);

            let brightness = ((pixel1[0] as u32 + pixel2[0] as u32) / 2
                + (pixel1[1] as u32 + pixel2[1] as u32) / 2
                + (pixel1[2] as u32 + pixel2[2] as u32) / 2)
                / 3;

            let idx = (brightness as usize * ASCII_CHARS.len()) / 256;
            line_str.push(ASCII_CHARS[idx.min(ASCII_CHARS.len() - 1)]);
        }
        lines.push(Line::from(line_str));
    }

    lines
}
