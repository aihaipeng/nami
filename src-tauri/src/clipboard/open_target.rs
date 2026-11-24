use std::{
    fmt::Write,
    path::{Path, PathBuf},
};

use reqwest::Url;

use super::MAX_FILE_COUNT;

#[derive(Debug, PartialEq, Eq)]
pub enum OpenTarget {
    Link(Url),
    Email(Url),
    File(PathBuf),
    ChooseFile,
}

impl OpenTarget {
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Link(_) => "link",
            Self::Email(_) => "email",
            Self::File(_) => "file",
            Self::ChooseFile => "files",
        }
    }
}

pub fn resolve_paths(
    paths: &[String],
    file_index: Option<usize>,
) -> Result<Option<OpenTarget>, String> {
    if paths.is_empty() || paths.len() > MAX_FILE_COUNT {
        return Err("storage_corrupt".into());
    }
    if file_index.is_none() && paths.len() > 1 {
        return Ok(Some(OpenTarget::ChooseFile));
    }
    let path = paths
        .get(file_index.unwrap_or(0))
        .ok_or("invalid_argument")?;
    let path = PathBuf::from(path);
    if path.is_absolute() && !path.to_string_lossy().contains('\0') {
        Ok(Some(OpenTarget::File(path)))
    } else {
        Err("storage_corrupt".into())
    }
}

pub fn resolve(
    kind: &str,
    text: &str,
    file_index: Option<usize>,
) -> Result<Option<OpenTarget>, String> {
    if kind == "files" {
        let paths: Vec<String> = serde_json::from_str(text).map_err(|_| "storage_corrupt")?;
        return resolve_paths(&paths, file_index);
    }
    if file_index.is_some() {
        return Err("invalid_argument".into());
    }
    if kind != "text" {
        return Ok(None);
    }
    let value = text.trim();
    if value.is_empty()
        || value
            .chars()
            .any(|c| c.is_control() || matches!(c, '\u{2028}' | '\u{2029}'))
    {
        return Ok(None);
    }
    let path = Path::new(value);
    if path.is_absolute() {
        return Ok(Some(OpenTarget::File(path.into())));
    }
    if value.chars().any(char::is_whitespace) || value.contains('\\') {
        return Ok(None);
    }
    let lower = value.to_ascii_lowercase();
    let prefixed;
    let address = if lower.starts_with("www.") {
        prefixed = format!("https://{value}");
        Some(prefixed.as_str())
    } else if lower.starts_with("http://") || lower.starts_with("https://") {
        Some(value)
    } else {
        None
    };
    if let Some(address) = address {
        return Ok(Url::parse(address)
            .ok()
            .filter(|url| {
                !address
                    .split_once("://")
                    .unwrap_or_default()
                    .1
                    .starts_with('/')
                    && url.has_host()
                    && url.username().is_empty()
                    && url.password().is_none()
            })
            .map(OpenTarget::Link));
    }
    let Some((local, domain)) = value.split_once('@') else {
        return Ok(None);
    };
    if local.len() > 64
        || domain.len() > 253
        || !domain.contains('.')
        || !local.split('.').all(|part| {
            !part.is_empty()
                && part
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"!#$%&'*+-/=?^_`{|}~".contains(&b))
        })
        || !domain.split('.').all(|label| {
            !label.is_empty()
                && label.len() <= 63
                && !label.starts_with('-')
                && !label.ends_with('-')
                && label
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-')
        })
    {
        return Ok(None);
    }
    let mut encoded = String::new();
    for byte in local.bytes() {
        if byte.is_ascii_alphanumeric() || b"-._~".contains(&byte) {
            encoded.push(char::from(byte));
        } else {
            let _ = write!(encoded, "%{byte:02X}");
        }
    }
    Ok(Url::parse(&format!("mailto:{encoded}@{domain}"))
        .ok()
        .map(OpenTarget::Email))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_only_single_web_or_email_targets_and_encodes_recipient() {
        let text = |value| resolve("text", value, None).unwrap();
        assert_eq!(
            text("  www.example.test/abc  "),
            Some(OpenTarget::Link(
                Url::parse("https://www.example.test/abc").unwrap()
            ))
        );
        assert_eq!(
            text("a+b?c%0a@example.test"),
            Some(OpenTarget::Email(
                Url::parse("mailto:a%2Bb%3Fc%250a@example.test").unwrap()
            ))
        );
        for value in [
            "example.test",
            "see https://example.test",
            "https://example.test\nappendix",
            "https:///example.test",
            "https://x@y.test",
            "file:///tmp/test",
            "javascript:alert(1)",
            "mailto:a@example.test?bcc=x",
            "a@example.test,b@example.test",
            "a@example.test?subject=x",
            "a..b@example.test",
        ] {
            assert_eq!(text(value), None, "{value}");
        }
    }

    #[test]
    fn file_groups_require_a_choice_and_never_accept_a_path_from_the_caller() {
        let root = std::env::temp_dir();
        let first = root.join("first.txt").to_string_lossy().into_owned();
        let second = root.join("second file.txt").to_string_lossy().into_owned();
        let photo = root.join("photo.png").to_string_lossy().into_owned();
        let bad = root.join("bad\0.png").to_string_lossy().into_owned();
        let files = serde_json::to_string(&[&first, &second]).unwrap();
        assert_eq!(
            resolve("files", &files, None).unwrap(),
            Some(OpenTarget::ChooseFile)
        );
        assert_eq!(
            resolve("files", &files, Some(1)).unwrap(),
            Some(OpenTarget::File(second.into()))
        );
        assert!(resolve("files", &files, Some(2)).is_err());
        assert!(resolve("files", r#"["relative.txt"]"#, None).is_err());
        assert!(resolve("text", &first, Some(0)).is_err());
        assert_eq!(
            resolve_paths(std::slice::from_ref(&photo), None).unwrap(),
            Some(OpenTarget::File(photo.clone().into()))
        );
        assert_eq!(
            resolve_paths(std::slice::from_ref(&photo), Some(0)).unwrap(),
            Some(OpenTarget::File(photo.clone().into()))
        );
        assert!(resolve_paths(&[], None).is_err());
        assert!(resolve_paths(&["relative.png".into()], None).is_err());
        assert!(resolve_paths(&[bad], None).is_err());
        assert!(resolve_paths(&[photo], Some(1)).is_err());
    }
}
