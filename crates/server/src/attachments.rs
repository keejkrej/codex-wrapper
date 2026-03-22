use std::path::{Path, PathBuf};

use anyhow::{anyhow, Result};
use base64::Engine;

use crate::model::ChatAttachment;
use crate::util::normalize_path;

const ATTACHMENT_EXTENSIONS: &[&str] = &[
    ".png", ".jpg", ".jpeg", ".gif", ".webp", ".bmp", ".svg", ".avif", ".bin",
];

pub(crate) fn normalize_attachment_relative_path(raw_relative_path: &str) -> Option<String> {
    if raw_relative_path.contains('\0') {
        return None;
    }

    let raw = raw_relative_path.replace('\\', "/");
    let trimmed = raw.trim().trim_start_matches('/');
    if trimmed.is_empty() {
        return None;
    }

    let mut segments = Vec::new();
    for segment in trimmed.split('/') {
        if segment.is_empty() || segment == "." {
            continue;
        }
        if segment == ".." {
            return None;
        }
        segments.push(segment);
    }

    if segments.is_empty() {
        return None;
    }

    Some(segments.join("/"))
}

pub(crate) fn resolve_attachment_relative_path(
    state_dir: &Path,
    relative_path: &str,
) -> Option<PathBuf> {
    let normalized_relative_path = normalize_attachment_relative_path(relative_path)?;
    let attachments_root = state_dir.join("attachments");
    let candidate = attachments_root.join(&normalized_relative_path);

    if !normalize_path(&candidate).starts_with(&normalize_path(&attachments_root)) {
        return None;
    }

    Some(candidate)
}

pub(crate) fn create_attachment_id(thread_id: &str) -> Option<String> {
    let segment = thread_id
        .trim()
        .to_lowercase()
        .chars()
        .map(|char| match char {
            'a'..='z' | '0'..='9' | '_' | '-' => char,
            _ => '-',
        })
        .collect::<String>()
        .split('-')
        .filter(|segment| !segment.is_empty())
        .collect::<Vec<_>>()
        .join("-");
    if segment.is_empty() {
        return None;
    }
    Some(format!("{segment}-{}", uuid::Uuid::new_v4()))
}

pub(crate) fn attachment_relative_path(attachment: &ChatAttachment) -> String {
    format!(
        "{}{}",
        attachment.id,
        infer_image_extension(&attachment.name, &attachment.mime_type)
    )
}

pub(crate) fn resolve_attachment_path(
    state_dir: &Path,
    attachment: &ChatAttachment,
) -> Option<PathBuf> {
    resolve_attachment_relative_path(state_dir, &attachment_relative_path(attachment))
}

pub(crate) fn resolve_attachment_path_by_id(
    state_dir: &Path,
    attachment_id: &str,
) -> Option<PathBuf> {
    let normalized_id = normalize_attachment_relative_path(attachment_id)?;
    if normalized_id.contains('/') || normalized_id.contains('.') {
        return None;
    }
    for extension in ATTACHMENT_EXTENSIONS {
        let candidate =
            resolve_attachment_relative_path(state_dir, &format!("{normalized_id}{extension}"))?;
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

pub(crate) fn parse_base64_data_url(data_url: &str) -> Result<(String, Vec<u8>)> {
    let (metadata, base64_payload) = data_url
        .split_once(',')
        .ok_or_else(|| anyhow!("data URL is missing a payload"))?;
    let metadata = metadata
        .strip_prefix("data:")
        .ok_or_else(|| anyhow!("data URL is missing the data: prefix"))?;
    let (mime_type, encoding) = metadata
        .split_once(';')
        .ok_or_else(|| anyhow!("data URL is missing an encoding marker"))?;
    if encoding != "base64" {
        return Err(anyhow!("data URL is not base64 encoded"));
    }

    let bytes = base64::engine::general_purpose::STANDARD
        .decode(base64_payload)
        .map_err(|error| anyhow!("failed to decode base64 payload: {error}"))?;
    Ok((mime_type.to_lowercase(), bytes))
}

pub(crate) fn infer_image_extension(name: &str, mime_type: &str) -> &'static str {
    let lower_name = name.to_lowercase();
    if lower_name.ends_with(".png") || mime_type.eq_ignore_ascii_case("image/png") {
        ".png"
    } else if lower_name.ends_with(".jpg")
        || lower_name.ends_with(".jpeg")
        || mime_type.eq_ignore_ascii_case("image/jpeg")
    {
        ".jpg"
    } else if lower_name.ends_with(".gif") || mime_type.eq_ignore_ascii_case("image/gif") {
        ".gif"
    } else if lower_name.ends_with(".webp") || mime_type.eq_ignore_ascii_case("image/webp") {
        ".webp"
    } else if lower_name.ends_with(".bmp") || mime_type.eq_ignore_ascii_case("image/bmp") {
        ".bmp"
    } else if lower_name.ends_with(".svg") || mime_type.eq_ignore_ascii_case("image/svg+xml") {
        ".svg"
    } else if lower_name.ends_with(".avif") || mime_type.eq_ignore_ascii_case("image/avif") {
        ".avif"
    } else {
        ".bin"
    }
}

#[cfg(test)]
mod tests {
    use super::{create_attachment_id, normalize_attachment_relative_path, parse_base64_data_url};

    #[test]
    fn rejects_escape_paths() {
        assert!(normalize_attachment_relative_path("../escape.png").is_none());
        assert!(normalize_attachment_relative_path("nested/../escape.png").is_none());
        assert_eq!(
            normalize_attachment_relative_path("thread folder/message folder/file.png"),
            Some("thread folder/message folder/file.png".to_string())
        );
    }

    #[test]
    fn creates_safe_attachment_ids() {
        let attachment_id = create_attachment_id("Thread.Foo/Unsafe Value").unwrap();
        assert!(attachment_id.starts_with("thread-foo-unsafe-value-"));
    }

    #[test]
    fn decodes_data_urls() {
        let (mime_type, bytes) = parse_base64_data_url("data:image/png;base64,aGVsbG8=").unwrap();
        assert_eq!(mime_type, "image/png");
        assert_eq!(bytes, b"hello");
    }
}
