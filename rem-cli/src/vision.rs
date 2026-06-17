use std::path::Path;

use crate::chat::ChatSession;
use crate::provider::Provider;
use crate::ui;

/// Handles the `/vision` command: analyzes an image with an optional prompt.
pub(crate) async fn handle_vision(client: &Provider, session: &mut ChatSession, input: &str) {
    let t = ui::theme::active();
    let parts: Vec<&str> = input.trim().splitn(2, ' ').collect();
    let image_path = parts.first().unwrap_or(&"");
    let prompt = parts.get(1).unwrap_or(&"Analyze this image");

    if image_path.is_empty() {
        println!(
            "{} {} usage: /vision <image-path> [prompt]",
            ui::theme::paint(&t, "accent", "\u{258C}", true),
            ui::theme::paint_warning(&t, "!")
        );
        return;
    }

    let path = if image_path.starts_with('/') {
        Path::new(image_path).to_path_buf()
    } else {
        let base = session
            .project_dir
            .clone()
            .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());
        base.join(image_path)
    };

    if !path.exists() {
        println!(
            "{} {} image not found: {}",
            ui::theme::paint(&t, "accent", "\u{258C}", true),
            ui::theme::paint_warning(&t, "\u{2717}"),
            path.display()
        );
        return;
    }

    if !is_image_file(&path) {
        println!(
            "{} {} not a supported image format (png, jpg, gif, webp, svg)",
            ui::theme::paint(&t, "accent", "\u{258C}", true),
            ui::theme::paint_warning(&t, "\u{2717}")
        );
        return;
    }

    let (mime, b64) = match encode_image(&path) {
        Ok(v) => v,
        Err(e) => {
            println!(
                "{} {} failed to read image: {}",
                ui::theme::paint(&t, "accent", "\u{258C}", true),
                ui::theme::paint_error_label(&t, "\u{2717}"),
                e
            );
            return;
        }
    };

    println!(
        "{} {} analyzing image ({} - {} bytes encoded)...",
        ui::theme::paint(&t, "accent", "\u{258C}", true),
        ui::theme::paint(&t, "accent", "\u{1F5BC}", true),
        mime,
        b64.len()
    );

    let result = client
        .complete_chat_stream_with_vision(prompt, "", "", &mime, &b64)
        .await;

    match result {
        Ok(text) => {
            println!("\n{}", text);
            session
                .history
                .push((format!("/vision {} {}", image_path, prompt), text));
        }
        Err(e) => {
            println!(
                "{} {} vision analysis failed: {}",
                ui::theme::paint(&t, "accent", "\u{258C}", true),
                ui::theme::paint_error_label(&t, "\u{2717}"),
                e
            );
        }
    }
}

/// Encodes a local image file to base64 with its MIME type.
pub(crate) fn encode_image(path: &Path) -> Result<(String, String), String> {
    let mime = detect_mime_type(path);
    let data = std::fs::read(path)
        .map_err(|e| format!("cannot read image '{}': {}", path.display(), e))?;
    let b64 = base64_encode(&data);
    Ok((mime, b64))
}

/// Returns the MIME type for common image extensions.
fn detect_mime_type(path: &Path) -> String {
    match path.extension().and_then(|e| e.to_str()).unwrap_or("") {
        "png" => "image/png".to_string(),
        "jpg" | "jpeg" => "image/jpeg".to_string(),
        "gif" => "image/gif".to_string(),
        "webp" => "image/webp".to_string(),
        "svg" => "image/svg+xml".to_string(),
        ext => format!("image/{}", ext),
    }
}

/// Returns true if a file path looks like an image.
pub(crate) fn is_image_file(path: &Path) -> bool {
    matches!(
        path.extension().and_then(|e| e.to_str()),
        Some("png" | "jpg" | "jpeg" | "gif" | "webp" | "svg" | "bmp")
    )
}

/// Builds a data URI for an OpenAI image_url content part.
pub(crate) fn build_image_data_uri(mime: &str, b64: &str) -> String {
    format!("data:{};base64,{}", mime, b64)
}

/// Simple base64 encoding (avoids adding a new crate dependency).
fn base64_encode(data: &[u8]) -> String {
    const CHARS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut result = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = chunk.get(1).copied().unwrap_or(0) as u32;
        let b2 = chunk.get(2).copied().unwrap_or(0) as u32;
        let triple = (b0 << 16) | (b1 << 8) | b2;
        result.push(CHARS[((triple >> 18) & 0x3F) as usize] as char);
        result.push(CHARS[((triple >> 12) & 0x3F) as usize] as char);
        result.push(if chunk.len() > 1 {
            CHARS[((triple >> 6) & 0x3F) as usize] as char
        } else {
            '='
        });
        result.push(if chunk.len() > 2 {
            CHARS[(triple & 0x3F) as usize] as char
        } else {
            '='
        });
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detect_mime_types() {
        assert_eq!(detect_mime_type(Path::new("img.png")), "image/png");
        assert_eq!(detect_mime_type(Path::new("photo.jpg")), "image/jpeg");
        assert_eq!(detect_mime_type(Path::new("pic.jpeg")), "image/jpeg");
        assert_eq!(detect_mime_type(Path::new("anim.gif")), "image/gif");
        assert_eq!(detect_mime_type(Path::new("graphic.webp")), "image/webp");
        assert_eq!(detect_mime_type(Path::new("icon.svg")), "image/svg+xml");
    }

    #[test]
    fn test_is_image_file() {
        assert!(is_image_file(Path::new("photo.png")));
        assert!(is_image_file(Path::new("photo.jpg")));
        assert!(is_image_file(Path::new("photo.jpeg")));
        assert!(is_image_file(Path::new("anim.gif")));
        assert!(is_image_file(Path::new("img.webp")));
        assert!(!is_image_file(Path::new("main.rs")));
        assert!(!is_image_file(Path::new("index.html")));
    }

    #[test]
    fn test_base64_encode_basic() {
        let encoded = base64_encode(b"hello");
        assert_eq!(encoded, "aGVsbG8=");
    }

    #[test]
    fn test_base64_encode_empty() {
        let encoded = base64_encode(b"");
        assert_eq!(encoded, "");
    }

    #[test]
    fn test_base64_padding() {
        let encoded = base64_encode(b"f");
        assert_eq!(encoded, "Zg==");
        let encoded = base64_encode(b"fo");
        assert_eq!(encoded, "Zm8=");
        let encoded = base64_encode(b"foo");
        assert_eq!(encoded, "Zm9v");
    }

    #[test]
    fn test_build_data_uri() {
        let uri = build_image_data_uri("image/png", "abc123");
        assert_eq!(uri, "data:image/png;base64,abc123");
    }
}
