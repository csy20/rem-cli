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
            .ctx
            .project_dir
            .clone()
            .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());
        match crate::types::resolve_safe_path(&base, image_path) {
            Some(p) => p,
            None => {
                println!(
                    "{} {} path traversal blocked: {}",
                    ui::theme::paint(&t, "accent", "\u{258C}", true),
                    ui::theme::paint_warning(&t, "\u{2717}"),
                    image_path
                );
                return;
            }
        }
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
                .history_mgr
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
    let data = std::fs::read(path).map_err(|e| format!("cannot read image '{}': {}", path.display(), e))?;
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

fn base64_encode(data: &[u8]) -> String {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.encode(data)
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
        let uri = format!("data:{};base64,{}", "image/png", "abc123");
        assert_eq!(uri, "data:image/png;base64,abc123");
    }
}
