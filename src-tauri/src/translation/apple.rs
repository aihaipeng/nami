use super::Language;

#[cfg(any(test, all(target_os = "macos", not(feature = "portable-ui"))))]
use super::{BatchError, SegmentResult};
#[cfg(any(test, all(target_os = "macos", not(feature = "portable-ui"))))]
use crate::model::LanguageDetection;
#[cfg(any(test, all(target_os = "macos", not(feature = "portable-ui"))))]
use serde::Deserialize;

pub(super) const UNAVAILABLE: &str = "Apple Translate is available on macOS 15 or later.";
// Verified against macOS Translation.framework LanguageAvailability().supportedLanguages (21 languages).
const LANGUAGES: &[Language] = &[
    Language {
        code: "ar",
        name: "Arabic",
        source: true,
        target: true,
    },
    Language {
        code: "zh-CN",
        name: "Chinese (Simplified)",
        source: true,
        target: true,
    },
    Language {
        code: "zh-TW",
        name: "Chinese (Traditional)",
        source: true,
        target: true,
    },
    Language {
        code: "nl",
        name: "Dutch",
        source: true,
        target: true,
    },
    Language {
        code: "en",
        name: "English",
        source: true,
        target: true,
    },
    Language {
        code: "en-GB",
        name: "English (UK)",
        source: true,
        target: true,
    },
    Language {
        code: "fr",
        name: "French",
        source: true,
        target: true,
    },
    Language {
        code: "de",
        name: "German",
        source: true,
        target: true,
    },
    Language {
        code: "hi",
        name: "Hindi",
        source: true,
        target: true,
    },
    Language {
        code: "id",
        name: "Indonesian",
        source: true,
        target: true,
    },
    Language {
        code: "it",
        name: "Italian",
        source: true,
        target: true,
    },
    Language {
        code: "ja",
        name: "Japanese",
        source: true,
        target: true,
    },
    Language {
        code: "ko",
        name: "Korean",
        source: true,
        target: true,
    },
    Language {
        code: "pl",
        name: "Polish",
        source: true,
        target: true,
    },
    Language {
        code: "pt",
        name: "Portuguese (Brazil)",
        source: true,
        target: true,
    },
    Language {
        code: "ru",
        name: "Russian",
        source: true,
        target: true,
    },
    Language {
        code: "es",
        name: "Spanish",
        source: true,
        target: true,
    },
    Language {
        code: "th",
        name: "Thai",
        source: true,
        target: true,
    },
    Language {
        code: "tr",
        name: "Turkish",
        source: true,
        target: true,
    },
    Language {
        code: "uk",
        name: "Ukrainian",
        source: true,
        target: true,
    },
    Language {
        code: "vi",
        name: "Vietnamese",
        source: true,
        target: true,
    },
];

pub fn languages() -> &'static [Language] {
    LANGUAGES
}

#[cfg(any(test, all(target_os = "macos", not(feature = "portable-ui"))))]
pub(super) fn language_code(code: &str) -> &str {
    match code {
        "auto" => "auto",
        "zh-CN" => "zh-Hans",
        "zh-TW" => "zh-Hant",
        "ar" => "ar-AE",
        other => other,
    }
}

#[cfg(any(test, all(target_os = "macos", not(feature = "portable-ui"))))]
pub(super) fn normalize_language(code: &str) -> String {
    let trimmed = code.trim();
    let lower = trimmed.to_ascii_lowercase();
    let canonical = match lower.as_str() {
        "zh" | "zh-hans" | "zh-cn" | "zh-chs" => "zh-CN",
        "zh-hant" | "zh-tw" | "zh-hk" | "zh-mo" | "zh-cht" => "zh-TW",
        "ar" | "ar-ae" => "ar",
        "en-gb" => "en-GB",
        "en" | "en-us" => "en",
        "pt" | "pt-br" => "pt",
        other => other,
    };
    if let Some(language) = LANGUAGES
        .iter()
        .find(|language| language.code.eq_ignore_ascii_case(canonical))
    {
        return language.code.to_string();
    }
    if let Some((primary, _)) = canonical.split_once('-') {
        if let Some(language) = LANGUAGES
            .iter()
            .find(|language| language.code.eq_ignore_ascii_case(primary))
        {
            return language.code.to_string();
        }
    }
    trimmed.to_string()
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg(any(test, all(target_os = "macos", not(feature = "portable-ui"))))]
struct BridgeItem {
    text: String,
    detected_language: Option<String>,
    score: Option<f64>,
}

#[cfg(any(test, all(target_os = "macos", not(feature = "portable-ui"))))]
#[derive(Deserialize)]
struct BridgeResponse {
    ok: bool,
    items: Option<Vec<BridgeItem>>,
    error: Option<String>,
    code: Option<String>,
}

#[cfg(any(test, all(target_os = "macos", not(feature = "portable-ui"))))]
pub(super) fn parse(
    json: &str,
    count: usize,
    source: &str,
) -> Result<Vec<SegmentResult>, BatchError> {
    let malformed =
        || BatchError::permanent("The translation service returned an invalid response.");
    let response: BridgeResponse = serde_json::from_str(json).map_err(|_| malformed())?;
    if !response.ok {
        if response.code.as_deref() == Some("cancelled") {
            return Err(BatchError::permanent("cancelled"));
        }
        return Err(BatchError::permanent(
            response
                .error
                .filter(|message| !message.trim().is_empty())
                .unwrap_or_else(|| "Apple Translate failed. Try again.".into()),
        ));
    }
    let items = response
        .items
        .filter(|items| items.len() == count)
        .ok_or_else(malformed)?;
    items
        .into_iter()
        .map(|item| {
            let detection = if source == "auto" {
                let raw = item
                    .detected_language
                    .as_deref()
                    .map(str::trim)
                    .filter(|code| !code.is_empty())
                    .ok_or_else(malformed)?;
                let score = match item.score {
                    None => None,
                    Some(value) if (0.0..=1.0).contains(&value) => Some(value),
                    Some(_) => return Err(malformed()),
                };
                Some(LanguageDetection {
                    raw_language: raw.to_string(),
                    language: normalize_language(raw),
                    score,
                })
            } else {
                None
            };
            Ok(SegmentResult {
                text: item.text,
                detection,
            })
        })
        .collect()
}

#[cfg(all(target_os = "macos", not(feature = "portable-ui")))]
mod native {
    use std::ffi::{c_char, c_void, CStr, CString};
    use std::sync::atomic::{AtomicU64, Ordering};
    use tokio::sync::oneshot;

    use super::{language_code, parse};
    use crate::translation::{BatchError, SegmentResult};

    static NEXT_REQUEST_ID: AtomicU64 = AtomicU64::new(1);

    extern "C" {
        fn nami_apple_translation_available() -> bool;
        fn nami_apple_translation_cancel(request_id: u64);
        fn nami_apple_translation_translate(
            request_id: u64,
            texts_json: *const c_char,
            source: *const c_char,
            target: *const c_char,
            callback: extern "C" fn(*mut c_void, *const c_char),
            context: *mut c_void,
        );
    }

    extern "C" fn translate_callback(context: *mut c_void, response_json: *const c_char) {
        if context.is_null() {
            return;
        }
        let sender = unsafe { Box::from_raw(context as *mut oneshot::Sender<String>) };
        let payload = if response_json.is_null() {
            String::new()
        } else {
            unsafe { CStr::from_ptr(response_json) }
                .to_string_lossy()
                .into_owned()
        };
        let _ = sender.send(payload);
    }

    struct CancelGuard {
        request_id: u64,
        completed: bool,
    }

    impl Drop for CancelGuard {
        fn drop(&mut self) {
            if !self.completed {
                unsafe {
                    nami_apple_translation_cancel(self.request_id);
                }
            }
        }
    }

    pub fn is_available() -> bool {
        unsafe { nami_apple_translation_available() }
    }

    pub async fn translate_batch(
        texts: &[&str],
        source: &str,
        target: &str,
    ) -> Result<Vec<SegmentResult>, BatchError> {
        if !is_available() {
            return Err(BatchError::permanent(super::UNAVAILABLE));
        }
        let encoded_texts = serde_json::to_string(texts)
            .map_err(|_| BatchError::permanent("Unable to encode Apple Translate input."))?;
        let texts_c = CString::new(encoded_texts)
            .map_err(|_| BatchError::permanent("Unable to encode Apple Translate input."))?;
        let source_c = CString::new(language_code(source))
            .map_err(|_| BatchError::permanent("Invalid source language for Apple Translate."))?;
        let target_c = CString::new(language_code(target))
            .map_err(|_| BatchError::permanent("Invalid target language for Apple Translate."))?;

        let request_id = NEXT_REQUEST_ID.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = oneshot::channel::<String>();
        let context = Box::into_raw(Box::new(tx)) as *mut c_void;
        let mut guard = CancelGuard {
            request_id,
            completed: false,
        };

        unsafe {
            nami_apple_translation_translate(
                request_id,
                texts_c.as_ptr(),
                source_c.as_ptr(),
                target_c.as_ptr(),
                translate_callback,
                context,
            );
        }

        let payload = rx
            .await
            .map_err(|_| BatchError::permanent("Apple Translate failed. Try again."))?;
        guard.completed = true;
        parse(&payload, texts.len(), source)
    }
}

#[cfg(any(not(target_os = "macos"), feature = "portable-ui"))]
mod native {
    use crate::translation::{BatchError, SegmentResult};

    pub fn is_available() -> bool {
        false
    }

    pub async fn translate_batch(
        _texts: &[&str],
        _source: &str,
        _target: &str,
    ) -> Result<Vec<SegmentResult>, BatchError> {
        Err(BatchError::permanent(super::UNAVAILABLE))
    }
}

pub fn is_available() -> bool {
    native::is_available()
}

pub(super) async fn translate_batch(
    texts: &[&str],
    source: &str,
    target: &str,
) -> Result<Vec<super::SegmentResult>, super::BatchError> {
    native::translate_batch(texts, source, target).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn catalog_contains_unique_round_trippable_apple_languages() {
        assert_eq!(languages().len(), 21);
        let mut codes = std::collections::HashSet::new();
        for language in languages() {
            assert!(codes.insert(language.code));
            assert!(language.source && language.target);
            assert!(!language.name.is_empty());
            assert_eq!(
                normalize_language(language_code(language.code)),
                language.code
            );
        }
        assert_eq!(language_code("auto"), "auto");
        assert_eq!(language_code("zh-CN"), "zh-Hans");
        assert_eq!(language_code("zh-TW"), "zh-Hant");
        assert_eq!(language_code("ar"), "ar-AE");
        assert_eq!(normalize_language("zh"), "zh-CN");
        assert_eq!(normalize_language("zh-TW"), "zh-TW");
        assert_eq!(normalize_language("ar-AE"), "ar");
        assert_eq!(normalize_language("en-GB"), "en-GB");
        assert_eq!(normalize_language("en-US"), "en");
        assert_eq!(normalize_language("pt-BR"), "pt");
    }

    #[test]
    fn parses_auto_and_manual_apple_bridge_responses() {
        let auto_payload = json!({
            "ok": true,
            "items": [
                {"text": "你好", "detectedLanguage": "en", "score": 0.95},
                {"text": "Hello", "detectedLanguage": "zh", "score": null}
            ]
        })
        .to_string();
        let rows = parse(&auto_payload, 2, "auto").unwrap();
        assert_eq!(rows[0].text, "你好");
        let first = rows[0].detection.as_ref().unwrap();
        assert_eq!(first.raw_language, "en");
        assert_eq!(first.language, "en");
        assert_eq!(first.score, Some(0.95));
        let second = rows[1].detection.as_ref().unwrap();
        assert_eq!(second.raw_language, "zh");
        assert_eq!(second.language, "zh-CN");
        assert_eq!(second.score, None);

        let manual_payload = json!({
            "ok": true,
            "items": [{"text": "世界", "detectedLanguage": null, "score": null}]
        })
        .to_string();
        let manual = parse(&manual_payload, 1, "en").unwrap();
        assert_eq!(manual[0].text, "世界");
        assert!(manual[0].detection.is_none());
    }

    #[test]
    fn maps_bridge_errors_and_rejects_malformed_payloads() {
        let not_installed = json!({
            "ok": false,
            "error": "Apple Translate language pack is not downloaded.",
            "code": "not_installed"
        })
        .to_string();
        let err = parse(&not_installed, 1, "auto").err().unwrap();
        assert!(!err.retry);
        assert!(err.message.contains("language pack"));

        let cancelled = json!({
            "ok": false,
            "error": "cancelled",
            "code": "cancelled"
        })
        .to_string();
        let err = parse(&cancelled, 1, "auto").err().unwrap();
        assert_eq!(err.message, "cancelled");

        assert!(parse("not-json", 1, "auto").is_err());
        assert!(parse(r#"{"ok":true,"items":[]}"#, 1, "auto").is_err());
        assert!(parse(
            r#"{"ok":true,"items":[{"text":"A","detectedLanguage":"en","score":1.5}]}"#,
            1,
            "auto"
        )
        .is_err());
    }
}
