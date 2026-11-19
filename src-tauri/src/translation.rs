use crate::shortcuts::{self, KeyBinding, TranslationShortcuts};
use reqwest::{Client, Response};
use serde::{Deserialize, Serialize};

use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

const MAX_RESPONSE_BYTES: usize = 2 * 1024 * 1024;
const TRANSLATION_RETRY_LIMIT: usize = 3;
const TRANSLATION_RETRY_DELAY: Duration = Duration::from_millis(300);
mod edge;
mod google;
mod text;

pub const STRATEGY_VERSION: u32 = 1;

#[derive(Serialize)]
pub struct Language {
    pub code: &'static str,
    pub name: &'static str,
    pub source: bool,
    pub target: bool,
}

struct SegmentResult {
    text: String,
    detection: Option<crate::model::LanguageDetection>,
}

pub struct TranslationResult {
    pub translations: Vec<String>,
    pub source_language: String,
    pub detections: Vec<crate::model::LanguageDetection>,
}

type BatchTranslation = Vec<SegmentResult>;
type BatchFuture<'a> = Pin<Box<dyn Future<Output = Result<BatchTranslation, String>> + Send + 'a>>;
pub fn languages(engine: &str) -> &'static [Language] {
    match engine {
        "google" => google::languages(),
        "edge" => edge::languages(),
        _ => &[],
    }
}

fn supports_language(engine: &str, code: &str, source: bool) -> bool {
    languages(engine).iter().any(|language| {
        language.code == code
            && if source {
                language.source
            } else {
                language.target
            }
    })
}

fn known_language(code: &str, source: bool) -> bool {
    ["google", "edge"]
        .iter()
        .any(|engine| supports_language(engine, code, source))
}

struct Lines<'a> {
    texts: Vec<&'a str>,
    #[cfg(test)]
    endings: Vec<&'a str>,
}

fn split_lines(text: &str) -> Lines<'_> {
    let bytes = text.as_bytes();
    let mut texts = Vec::new();
    #[cfg(test)]
    let mut endings = Vec::new();
    let mut start = 0;
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'\r' || bytes[index] == b'\n' {
            let end = index;
            index += 1;
            if bytes[end] == b'\r' && bytes.get(index) == Some(&b'\n') {
                index += 1;
            }
            texts.push(&text[start..end]);
            #[cfg(test)]
            endings.push(&text[end..index]);
            start = index;
        } else {
            index += 1;
        }
    }
    texts.push(&text[start..]);
    #[cfg(test)]
    endings.push("");
    Lines {
        texts,
        #[cfg(test)]
        endings,
    }
}

pub fn validate_text(text: &str) -> Result<(), String> {
    if text.encode_utf16().count() > 100_000 {
        return Err("Text exceeds 100,000 UTF-16 characters.".into());
    }
    let lines = split_lines(text);
    if lines.texts.len() > 2_000 {
        return Err("Text exceeds 2,000 lines.".into());
    }
    if lines
        .texts
        .iter()
        .any(|line| line.encode_utf16().count() > 5_000)
    {
        return Err("A line exceeds 5,000 UTF-16 characters. Split it into shorter lines.".into());
    }
    Ok(())
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Preferences {
    pub engine: String,
    pub source_language: String,
    pub primary_language: String,
    pub secondary_language: String,
    pub proxy_url: String,
    pub proxy_enabled: bool,
    pub selection_enabled: bool,
    #[serde(default)]
    pub disabled_shortcuts: Vec<shortcuts::ShortcutAction>,
    pub shortcuts: TranslationShortcuts,
}

impl Preferences {
    pub fn defaults(primary: &str) -> Self {
        Self {
            engine: "edge".into(),
            source_language: "auto".into(),
            primary_language: "en".into(),
            secondary_language: "zh-CN".into(),
            proxy_url: String::new(),
            proxy_enabled: false,
            selection_enabled: true,
            disabled_shortcuts: Vec::new(),
            shortcuts: TranslationShortcuts::defaults(primary),
        }
    }

    pub fn from_store(read: impl Fn(&str) -> Option<String>, primary: &str) -> Self {
        let mut preferences = Self::defaults(primary);
        if let Some(engine) = read("translation_engine") {
            if matches!(engine.as_str(), "edge" | "google") {
                preferences.engine = engine;
            }
        }
        if let Some(source) = read("translation_source") {
            if source == "auto" || known_language(&source, true) {
                preferences.source_language = source;
            }
        }
        if let Some(primary_language) = read("translation_primary") {
            if known_language(&primary_language, false) {
                preferences.primary_language = primary_language;
            }
        }
        if let Some(secondary) = read("translation_secondary") {
            if secondary == "none" || known_language(&secondary, false) {
                preferences.secondary_language = secondary;
            }
        }
        let stored_proxy = read("translation_proxy_url").unwrap_or_default();
        let stored_proxy = stored_proxy.trim();
        if stored_proxy.chars().count() <= 2_048 && !proxy_contains_credentials(stored_proxy) {
            preferences.proxy_url = stored_proxy.to_string();
        }
        if read("translation_proxy_enabled").as_deref() == Some("true")
            && !proxy_contains_credentials(stored_proxy)
        {
            preferences.proxy_enabled = true;
        }
        if let Some(enabled) = read("translation_selection_enabled") {
            preferences.selection_enabled = enabled == "true";
        }
        if let Some(disabled) = read("translation_disabled_shortcuts")
            .and_then(|value| serde_json::from_str(&value).ok())
        {
            preferences.disabled_shortcuts = disabled;
        }
        preferences.shortcuts = TranslationShortcuts::from_store(read, primary);
        preferences
    }

    pub fn apply(&mut self, patch: PreferencePatch) -> Result<bool, String> {
        let mut engine_changed = false;
        if let Some(engine) = patch.engine {
            if !matches!(engine.as_str(), "edge" | "google") {
                return Err("invalid_argument".into());
            }
            engine_changed = self.engine != engine;
            self.engine = engine;
        }
        if let Some(source) = patch.source_language {
            if source != "auto" && !supports_language(&self.engine, &source, true) {
                return Err("invalid_argument".into());
            }
            self.source_language = source;
        }
        if let Some(primary) = patch.primary_language {
            if !supports_language(&self.engine, &primary, false) {
                return Err("invalid_argument".into());
            }
            self.primary_language = primary;
        }
        if let Some(secondary) = patch.secondary_language {
            if secondary != "none" && !supports_language(&self.engine, &secondary, false) {
                return Err("invalid_argument".into());
            }
            self.secondary_language = secondary;
        }
        if patch.proxy_url.is_some() || patch.proxy_enabled.is_some() {
            let url = patch
                .proxy_url
                .as_deref()
                .map(str::trim)
                .unwrap_or(&self.proxy_url);
            let enabled = patch.proxy_enabled.unwrap_or(self.proxy_enabled);
            if enabled || (patch.proxy_url.is_some() && !url.is_empty()) {
                if proxy_contains_credentials(url) {
                    return Err("proxy_credentials".into());
                }
                if url.chars().count() > 2_048 || configured_proxy(url, true).is_err() {
                    return Err("proxy_url".into());
                }
            }
            self.proxy_url = url.to_string();
            self.proxy_enabled = enabled;
        }
        if let Some(enabled) = patch.selection_enabled {
            self.selection_enabled = enabled;
        }
        Ok(engine_changed)
    }

    pub fn set_shortcut(
        &mut self,
        action: &str,
        binding: Option<KeyBinding>,
        clipboard: Option<&KeyBinding>,
    ) -> Result<(), String> {
        let action = shortcuts::ShortcutAction::parse(action)?;
        self.shortcuts = shortcuts::assign_shortcut(&self.shortcuts, action, binding, clipboard)?;
        Ok(())
    }

    pub fn set_shortcut_enabled(&mut self, action: &str, enabled: bool) -> Result<(), String> {
        let action = shortcuts::ShortcutAction::parse(action)?;
        if action == shortcuts::ShortcutAction::Selection {
            self.selection_enabled = enabled;
        } else {
            self.disabled_shortcuts
                .retain(|candidate| *candidate != action);
            if !enabled {
                self.disabled_shortcuts.push(action);
            }
        }
        Ok(())
    }

    pub fn entries(&self) -> Vec<(&'static str, String)> {
        vec![
            ("translation_engine", self.engine.clone()),
            ("translation_source", self.source_language.clone()),
            ("translation_primary", self.primary_language.clone()),
            ("translation_secondary", self.secondary_language.clone()),
            ("translation_proxy_url", self.proxy_url.clone()),
            (
                "translation_proxy_enabled",
                if self.proxy_enabled { "true" } else { "false" }.into(),
            ),
            (
                "translation_selection_enabled",
                if self.selection_enabled {
                    "true"
                } else {
                    "false"
                }
                .into(),
            ),
            (
                "translation_disabled_shortcuts",
                serde_json::to_string(&self.disabled_shortcuts).unwrap_or_else(|_| "[]".into()),
            ),
            (
                "translation_shortcuts",
                serde_json::to_string(&self.shortcuts).unwrap_or_else(|_| "{}".into()),
            ),
        ]
    }
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PreferencePatch {
    pub engine: Option<String>,
    pub source_language: Option<String>,
    pub primary_language: Option<String>,
    pub secondary_language: Option<String>,
    pub proxy_url: Option<String>,
    pub proxy_enabled: Option<bool>,
    pub selection_enabled: Option<bool>,
}

pub fn validate_options(engine: &str, source: &str, targets: &[String]) -> Result<(), String> {
    if !matches!(engine, "edge" | "google") {
        return Err("Choose a supported translation engine.".into());
    }
    if source != "auto" && !supports_language(engine, source, true) {
        return Err("Choose a supported source language.".into());
    }
    if !(1..=2).contains(&targets.len())
        || targets
            .iter()
            .any(|target| !supports_language(engine, target, false))
    {
        return Err("Choose one or two supported target languages.".into());
    }
    Ok(())
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TranslationReuse {
    pub record_id: String,
    pub target_index: Option<usize>,
}

pub fn source_app_for_translation(
    text: &str,
    previous: Option<&crate::model::HistoryRecord>,
    captured: Option<serde_json::Value>,
) -> Option<serde_json::Value> {
    match previous.filter(|record| record.text == text) {
        Some(record) => record.source_app.clone(),
        None => captured,
    }
}

pub fn reusable_results(
    engine: &str,
    text: &str,
    source: &str,
    targets: &[String],
    previous: Option<&crate::model::HistoryRecord>,
    refresh_target: Option<usize>,
) -> Vec<Option<String>> {
    targets
        .iter()
        .enumerate()
        .map(|(index, target)| {
            previous
                .filter(|_| refresh_target.is_some_and(|changed| changed < 2 && index != changed))
                .filter(|record| record.text == text)
                .filter(|record| {
                    record.translation.as_ref().is_some_and(|metadata| {
                        metadata.engine == engine
                            && metadata.requested_source == source
                            && metadata.strategy_version == STRATEGY_VERSION
                    })
                })
                .filter(|record| record.target_languages.get(index) == Some(target))
                .and_then(|record| record.translations.get(index))
                .cloned()
        })
        .collect()
}

fn batches(lines: &[&str]) -> Vec<Vec<usize>> {
    let mut result = Vec::new();
    let mut batch = Vec::new();
    let mut units = 0;
    for (index, line) in lines.iter().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let next = line.encode_utf16().count();
        if !batch.is_empty() && (batch.len() == 64 || units + next > 12_000) {
            result.push(std::mem::take(&mut batch));
            units = 0;
        }
        batch.push(index);
        units += next;
    }
    if !batch.is_empty() {
        result.push(batch);
    }
    result
}

fn matched_language(codes: &[Option<String>]) -> Option<String> {
    let mut language: Option<&str> = None;
    for code in codes
        .iter()
        .flatten()
        .filter(|code| !matches!(code.as_str(), "auto" | "und"))
    {
        match language {
            None => language = Some(code),
            Some(previous) if previous != code => return Some("mul".into()),
            _ => {}
        }
    }
    language.map(str::to_string)
}

fn network_error(error: reqwest::Error) -> String {
    if error.is_timeout() {
        "The translation request timed out. Try again.".into()
    } else {
        "Unable to connect to the translation service. Check your network.".into()
    }
}

async fn response_bytes(mut response: Response) -> Result<Vec<u8>, String> {
    match response.status().as_u16() {
        200..=299 => {}
        429 => return Err("Too many translation requests. Try again later.".into()),
        401 | 403 => return Err("The translation service is temporarily unavailable.".into()),
        code => {
            return Err(format!(
                "The translation service request failed (HTTP {code}). Try again."
            ))
        }
    }
    let oversized = || "The translation response exceeds the supported size.".to_string();
    if response
        .content_length()
        .is_some_and(|length| length > MAX_RESPONSE_BYTES as u64)
    {
        return Err(oversized());
    }
    let mut data = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(network_error)? {
        if chunk.len() > MAX_RESPONSE_BYTES - data.len() {
            return Err(oversized());
        }
        data.extend_from_slice(&chunk);
    }
    Ok(data)
}

#[cfg(test)]
fn join_lines(translations: &[String], endings: &[&str]) -> String {
    let mut result = String::new();
    for (translation, ending) in translations.iter().zip(endings) {
        result.push_str(translation);
        result.push_str(ending);
    }
    result
}

async fn with_translation_retries<'a, T: 'a>(
    delay: Duration,
    mut operation: impl FnMut() -> Pin<Box<dyn Future<Output = Result<T, String>> + Send + 'a>>,
) -> Result<T, String> {
    let mut last_error = String::new();
    for attempt in 0..=TRANSLATION_RETRY_LIMIT {
        if attempt > 0 {
            tokio::time::sleep(delay).await;
        }
        match operation().await {
            Ok(value) => return Ok(value),
            Err(error) => last_error = error,
        }
    }
    Err(last_error)
}

fn fetch_batch<'a>(
    client: &'a Client,
    engine: &'a str,
    active: &'a [&str],
    source: &'a str,
    target: &'a str,
) -> BatchFuture<'a> {
    Box::pin(async move {
        let request = match engine {
            "google" => google::request(client, active, source, target),
            "edge" => edge::request(client, active, source, target),
            _ => return Err("Choose a supported translation engine.".into()),
        };
        let response = request.send().await.map_err(network_error)?;
        let data = response_bytes(response).await?;
        match engine {
            "google" => google::parse(&data, active.len(), source),
            "edge" => edge::parse(&data, active.len(), source),
            _ => unreachable!(),
        }
    })
}

pub fn proxy_contains_credentials(url: &str) -> bool {
    let trimmed = url.trim();
    if trimmed
        .split_once("://")
        .map(|(_, rest)| rest.split(['/', '?', '#']).next().unwrap_or(rest))
        .is_some_and(|authority| authority.contains('@'))
    {
        return true;
    }
    reqwest::Url::parse(trimmed)
        .is_ok_and(|parsed| !parsed.username().is_empty() || parsed.password().is_some())
}

pub fn configured_proxy(url: &str, enabled: bool) -> Result<Option<reqwest::Proxy>, String> {
    if !enabled {
        return Ok(None);
    }
    let url = url.trim();
    if proxy_contains_credentials(url) {
        return Err("Proxy URLs cannot include a username or password.".into());
    }
    if url.is_empty() {
        return Err("Enter a valid HTTP or HTTPS proxy URL.".into());
    }
    let parsed = reqwest::Url::parse(url)
        .map_err(|_| "Enter a valid HTTP or HTTPS proxy URL.".to_string())?;
    let path_ok = parsed.path().is_empty() || parsed.path() == "/";
    if !matches!(parsed.scheme(), "http" | "https")
        || parsed.host_str().is_none_or(str::is_empty)
        || !path_ok
        || parsed.query().is_some()
        || parsed.fragment().is_some()
    {
        return Err("Enter a valid HTTP or HTTPS proxy URL.".into());
    }
    reqwest::Proxy::all(parsed)
        .map(Some)
        .map_err(|_| "Enter a valid HTTP or HTTPS proxy URL.".into())
}

fn translation_client(proxy_url: &str, proxy_enabled: bool) -> Result<Client, String> {
    let mut builder = Client::builder()
        .user_agent("Nami/0.1")
        .timeout(Duration::from_secs(20))
        .redirect(reqwest::redirect::Policy::none());
    if let Some(proxy) = configured_proxy(proxy_url, proxy_enabled)? {
        builder = builder.proxy(proxy);
    }
    builder
        .build()
        .map_err(|_| "Unable to initialize the translation service.".into())
}

fn context_groups<'a>(
    input: &'a str,
    units: &[text::Unit<'a>],
    detections: &[Option<crate::model::LanguageDetection>],
) -> Vec<(text::Unit<'a>, std::ops::Range<usize>)> {
    let mut groups = Vec::new();
    let mut cursor = 0;
    for line in text::units(input, false)
        .into_iter()
        .filter(|unit| unit.translate)
    {
        let start = line.original.as_ptr() as usize;
        let end = start + line.original.len();
        while cursor < units.len() && (units[cursor].original.as_ptr() as usize) < start {
            cursor += 1;
        }
        let first = cursor;
        while cursor < units.len() && (units[cursor].original.as_ptr() as usize) < end {
            cursor += 1;
        }
        let range = first..cursor;
        let mut language: Option<&str> = None;
        let mut count = 0;
        let same_language = range
            .clone()
            .filter(|index| units[*index].translate)
            .all(|index| {
                let Some(detection) = detections[index].as_ref() else {
                    return false;
                };
                if detection.score == Some(0.0)
                    || matches!(detection.language.as_str(), "auto" | "und")
                {
                    return false;
                }
                count += 1;
                match language {
                    Some(previous) => previous == detection.language,
                    None => {
                        language = Some(&detection.language);
                        true
                    }
                }
            });
        if same_language && count > 1 {
            groups.push((line, range));
        }
    }
    groups
}

pub async fn translate(
    engine: &str,
    input: &str,
    source: &str,
    targets: &[String],
    proxy_url: &str,
    proxy_enabled: bool,
) -> Result<TranslationResult, String> {
    validate_text(input)?;
    validate_options(engine, source, targets)?;
    let units = text::units(input, source == "auto");
    let prepared: Vec<&str> = units
        .iter()
        .map(|unit| {
            if unit.translate {
                unit.prepared.as_str()
            } else {
                ""
            }
        })
        .collect();
    let batch_plan = batches(&prepared);
    if batch_plan.is_empty() {
        return Ok(TranslationResult {
            translations: targets.iter().map(|_| input.to_string()).collect(),
            source_language: if source == "auto" {
                "und".into()
            } else {
                source.into()
            },
            detections: Vec::new(),
        });
    }
    let client = translation_client(proxy_url, proxy_enabled)?;
    let mut results = Vec::with_capacity(targets.len());
    let mut detections = Vec::new();
    for (target_index, target) in targets.iter().enumerate() {
        let mut target_detections = vec![None; units.len()];
        let mut translated_units: Vec<String> =
            units.iter().map(|unit| unit.original.to_string()).collect();
        for batch in &batch_plan {
            let active: Vec<&str> = batch.iter().map(|index| prepared[*index]).collect();
            let translated = with_translation_retries(TRANSLATION_RETRY_DELAY, || {
                fetch_batch(&client, engine, &active, source, target)
            })
            .await?;
            for (index, result) in batch.iter().zip(translated) {
                translated_units[*index] = units[*index].restore(&result.text)?;
                target_detections[*index] = result.detection;
            }
        }
        if source == "auto" {
            // Rejoin same-language sentences to retain pronouns and paragraph context.
            let groups = context_groups(input, &units, &target_detections);
            let prepared: Vec<&str> = groups
                .iter()
                .map(|(unit, _)| unit.prepared.as_str())
                .collect();
            for batch in batches(&prepared) {
                let active: Vec<&str> = batch.iter().map(|index| prepared[*index]).collect();
                let contextual = with_translation_retries(TRANSLATION_RETRY_DELAY, || {
                    fetch_batch(&client, engine, &active, source, target)
                })
                .await?;
                for (index, result) in batch.iter().zip(contextual) {
                    let (unit, range) = &groups[*index];
                    let restored = unit.restore(&result.text)?;
                    for index in range.clone() {
                        translated_units[index].clear();
                        target_detections[index] = None;
                    }
                    translated_units[range.start] = restored;
                    target_detections[range.start] = result.detection;
                }
            }
        }
        if target_index == 0 {
            detections.extend(target_detections.into_iter().flatten());
        }
        results.push(translated_units.concat());
    }
    let resolved = if source == "auto" {
        matched_language(
            &detections
                .iter()
                .map(|value| Some(value.language.clone()))
                .collect::<Vec<_>>(),
        )
        .unwrap_or_else(|| "und".into())
    } else {
        source.to_string()
    };
    Ok(TranslationResult {
        translations: results,
        source_language: resolved,
        detections,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disabled_shortcuts_round_trip_without_changing_bindings() {
        let mut preferences = Preferences::defaults("meta");
        let bindings = preferences.shortcuts.clone();
        preferences
            .set_shortcut_enabled("translate", false)
            .unwrap();
        preferences
            .set_shortcut_enabled("selection", false)
            .unwrap();
        let entries = preferences.entries();
        let mut loaded = Preferences::from_store(
            |key| {
                entries
                    .iter()
                    .find(|(name, _)| *name == key)
                    .map(|(_, value)| value.clone())
            },
            "meta",
        );
        assert_eq!(loaded, preferences);
        assert_eq!(
            loaded.disabled_shortcuts,
            vec![shortcuts::ShortcutAction::Translate]
        );
        assert!(!loaded.selection_enabled);
        loaded.set_shortcut_enabled("translate", true).unwrap();
        assert!(loaded.disabled_shortcuts.is_empty());
        assert_eq!(loaded.shortcuts, bindings);
    }

    #[test]
    fn proxy_accepts_http_urls_and_rejects_other_forms() {
        assert!(configured_proxy("http://127.0.0.1:7890", true).is_ok());
        assert!(configured_proxy("https://proxy.example.com:8443", true).is_ok());
        assert!(configured_proxy("http://proxy.example.com:8080", false)
            .unwrap()
            .is_none());
        assert!(configured_proxy("https://user:pass@proxy.example.com:8443", true).is_err());
        assert!(configured_proxy("socks5://127.0.0.1:1080", true).is_err());
        assert!(configured_proxy("http://proxy.example.com/path", true).is_err());
        assert!(configured_proxy("http://proxy.example.com:8080?x=1", true).is_err());
        assert!(configured_proxy("", true).is_err());
    }

    #[test]
    fn proxy_preferences_reject_invalid_changes_before_they_can_replace_active_settings() {
        let mut preferences = Preferences::defaults("meta");
        let patch = |url: Option<&str>, enabled| PreferencePatch {
            proxy_url: url.map(str::to_string),
            proxy_enabled: enabled,
            ..PreferencePatch::default()
        };
        assert_eq!(
            preferences.apply(patch(None, Some(true))),
            Err("proxy_url".into())
        );
        assert_eq!(
            preferences.apply(patch(Some("socks5://127.0.0.1:1080"), None)),
            Err("proxy_url".into())
        );
        assert!(preferences.proxy_url.is_empty());
        assert!(!preferences.proxy_enabled);
        preferences
            .apply(patch(Some(" https://proxy.example.com:8443 "), Some(true)))
            .unwrap();
        for url in [
            "",
            "http://proxy.example.com/path",
            "http://proxy.example.com?query",
        ] {
            assert_eq!(
                preferences.apply(patch(Some(url), None)),
                Err("proxy_url".into())
            );
        }
        assert_eq!(
            preferences.apply(patch(Some("http://user:pass@127.0.0.1:7890"), None)),
            Err("proxy_credentials".into())
        );
        assert_eq!(preferences.proxy_url, "https://proxy.example.com:8443");
        assert!(preferences.proxy_enabled);
        preferences.proxy_url = "legacy-invalid".into();
        preferences.apply(patch(None, Some(false))).unwrap();
        preferences.apply(patch(Some(""), None)).unwrap();
        assert!(!preferences.proxy_enabled);
        assert!(preferences.proxy_url.is_empty());
    }

    #[test]
    fn stored_preferences_keep_valid_values_and_ignore_corrupt_ones() {
        let primary = "meta";
        let defaults = Preferences::defaults(primary);
        assert_eq!(defaults.engine, "edge");
        assert_eq!(defaults.shortcuts.selection.as_ref().unwrap().code, "KeyT");
        let mut stored = defaults.clone();
        stored.engine = "google".into();
        stored.primary_language = "de".into();
        let entries = stored.entries();
        let read = |key: &str| {
            entries
                .iter()
                .find(|(name, _)| *name == key)
                .map(|(_, value)| value.clone())
        };
        assert_eq!(
            Preferences::from_store(read, primary).primary_language,
            "de"
        );
        let mut corrupt = stored.clone();
        assert!(corrupt
            .apply(PreferencePatch {
                engine: Some("deepl".into()),
                ..PreferencePatch::default()
            })
            .is_err());
        assert_eq!(corrupt.engine, "google");
        corrupt.set_shortcut("translate", None, None).unwrap();
        assert_eq!(corrupt.shortcuts.translate, None);
        assert!(corrupt
            .set_shortcut(
                "translate",
                Some(crate::shortcuts::KeyBinding::new("KeyT", &["meta"])),
                None,
            )
            .is_err());
        assert!(corrupt
            .apply(PreferencePatch {
                proxy_url: Some("http://user:pass@127.0.0.1:7890".into()),
                ..PreferencePatch::default()
            })
            .is_err());
        assert!(!corrupt.proxy_url.contains('@'));
        let secret = [
            (
                "translation_proxy_url",
                "http://user:pass@127.0.0.1:7890".to_string(),
            ),
            ("translation_proxy_enabled", "true".to_string()),
        ];
        let loaded = Preferences::from_store(
            |key| {
                secret
                    .iter()
                    .find(|(name, _)| *name == key)
                    .map(|(_, value)| value.clone())
            },
            primary,
        );
        assert!(loaded.proxy_url.is_empty());
        assert!(!loaded.proxy_enabled);
    }

    #[test]
    fn language_switch_reuses_only_unchanged_columns() {
        let record = crate::model::HistoryRecord {
            title: String::new(),
            id: "previous".into(),
            text: "Synthetic".into(),
            source_language: "auto".into(),
            target_languages: vec!["en".into(), "zh-CN".into()],
            translations: vec!["English".into(), "Chinese".into()],
            date: "test".into(),
            is_favorite: false,
            is_pinned: false,
            source_app: None,
            translation: Some(crate::model::TranslationMetadata {
                requested_source: "auto".into(),
                engine: "edge".into(),
                strategy_version: STRATEGY_VERSION,
                detections: Vec::new(),
            }),
        };
        assert_eq!(
            reusable_results(
                "edge",
                "Synthetic",
                "auto",
                &record.target_languages,
                Some(&record),
                Some(0)
            ),
            vec![None, Some("Chinese".into())]
        );
        assert_eq!(
            reusable_results(
                "edge",
                "Synthetic",
                "auto",
                &record.target_languages,
                Some(&record),
                None
            ),
            vec![None, None]
        );
        let app = serde_json::json!({"bundleId": "synthetic.source", "name": "Synthetic Source", "iconDataUrl": "data:image/png;base64,c3ludGhldGlj"});
        let mut sourced = record.clone();
        sourced.source_app = Some(app.clone());
        assert_eq!(
            source_app_for_translation("Synthetic", Some(&sourced), None),
            Some(app.clone())
        );
        assert_eq!(
            source_app_for_translation("Edited", Some(&sourced), None),
            None
        );
        assert_eq!(
            source_app_for_translation("Synthetic", None, Some(app.clone())),
            Some(app.clone())
        );
        assert_eq!(
            source_app_for_translation("Synthetic", Some(&record), Some(app)),
            None
        );
        let full_refresh: TranslationReuse =
            serde_json::from_str(r#"{"recordId":"previous"}"#).unwrap();
        assert_eq!(full_refresh.target_index, None);
        assert_eq!(
            reusable_results(
                "edge",
                "Synthetic",
                "auto",
                &record.target_languages,
                Some(&sourced),
                full_refresh.target_index
            ),
            vec![None, None]
        );
        let targets = vec!["de".into(), "zh-CN".into()];
        assert_eq!(
            reusable_results(
                "edge",
                "Synthetic",
                "auto",
                &targets,
                Some(&record),
                Some(0)
            ),
            vec![None, Some("Chinese".into())]
        );
        assert_eq!(
            reusable_results("edge", "Synthetic", "de", &targets, Some(&record), Some(0)),
            vec![None, None]
        );
        assert_eq!(
            reusable_results("edge", "Edited", "auto", &targets, Some(&record), Some(0)),
            vec![None, None]
        );
        assert_eq!(
            reusable_results("edge", "Synthetic", "auto", &targets, None, Some(0)),
            vec![None, None]
        );
        assert_eq!(
            reusable_results(
                "edge",
                "Synthetic",
                "auto",
                &["en".into()],
                Some(&record),
                Some(1)
            ),
            vec![Some("English".into())]
        );
        assert_eq!(
            reusable_results(
                "edge",
                "Synthetic",
                "auto",
                &["en".into(), "fr".into()],
                Some(&record),
                Some(1)
            ),
            vec![Some("English".into()), None]
        );
    }

    #[test]
    fn validates_utf16_lines_and_existing_language_catalog() {
        assert!(validate_text(&"😀".repeat(2_500)).is_ok());
        assert!(validate_text(&"😀".repeat(2_501)).is_err());
        assert!(validate_text(&"\n".repeat(2_000)).is_err());
        assert!(validate_text(&format!("{}\n", "x".repeat(4_999)).repeat(20)).is_ok());
        assert!(validate_text(&format!("{}\n", "x".repeat(5_000)).repeat(20)).is_err());
        assert!(validate_options("edge", "auto", &["zh-CN".into()]).is_ok());
        assert!(validate_options("google", "en", &["ja".into(), "zh-TW".into()]).is_ok());
        assert!(validate_options("other", "auto", &["en".into()]).is_err());
        assert!(validate_options("edge", "none", &["en".into()]).is_err());
        assert!(validate_options("edge", "auto", &[]).is_err());
        assert!(validate_options("edge", "auto", &["auto".into()]).is_err());
        assert!(
            validate_options("edge", "auto", &["en".into(), "en".into(), "en".into()]).is_err()
        );
    }

    #[test]
    fn batches_at_row_and_utf16_limits_without_blank_requests() {
        let lines = vec!["hello"; 65];
        assert_eq!(
            batches(&lines).iter().map(Vec::len).collect::<Vec<_>>(),
            [64, 1]
        );
        let long = "😀".repeat(2_500);
        assert_eq!(batches(&[&long, "  ", &long, &long]), [vec![0, 2], vec![3]]);
        assert!(batches(&["", "  ", "\t"]).is_empty());
    }

    #[test]
    fn preserves_mixed_newlines_trailing_lines_and_blank_whitespace() {
        let text = "First\r\n \t\nSecond\rThird\n";
        let lines = split_lines(text);
        assert_eq!(lines.texts, ["First", " \t", "Second", "Third", ""]);
        assert_eq!(lines.endings, ["\r\n", "\n", "\r", "\n", ""]);
        let mut translated: Vec<String> = lines.texts.iter().map(|line| line.to_string()).collect();
        for index in batches(&lines.texts).into_iter().flatten() {
            translated[index] = "译文".into();
        }
        assert_eq!(
            join_lines(&translated, &lines.endings),
            "译文\r\n \t\n译文\r译文\n"
        );
        let empty = split_lines("");
        assert_eq!(empty.texts, [""]);
    }

    #[tokio::test]
    async fn retries_transient_translation_failures_before_reporting() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        assert_eq!(TRANSLATION_RETRY_LIMIT, 3);
        assert_eq!(TRANSLATION_RETRY_DELAY, Duration::from_millis(300));
        let calls = AtomicUsize::new(0);
        let result = with_translation_retries(Duration::ZERO, || {
            let attempt = calls.fetch_add(1, Ordering::SeqCst);
            Box::pin(async move {
                if attempt < TRANSLATION_RETRY_LIMIT {
                    Err(format!("transient {attempt}"))
                } else {
                    Ok(attempt)
                }
            })
        })
        .await
        .unwrap();
        assert_eq!(result, TRANSLATION_RETRY_LIMIT);
        assert_eq!(calls.load(Ordering::SeqCst), TRANSLATION_RETRY_LIMIT + 1);

        calls.store(0, Ordering::SeqCst);
        let error = with_translation_retries(Duration::ZERO, || {
            let attempt = calls.fetch_add(1, Ordering::SeqCst);
            Box::pin(async move { Err::<usize, _>(format!("failure {attempt}")) })
        })
        .await
        .unwrap_err();
        assert_eq!(error, format!("failure {TRANSLATION_RETRY_LIMIT}"));
        assert_eq!(calls.load(Ordering::SeqCst), TRANSLATION_RETRY_LIMIT + 1);

        calls.store(0, Ordering::SeqCst);
        let ready = with_translation_retries(Duration::ZERO, || {
            calls.fetch_add(1, Ordering::SeqCst);
            Box::pin(async { Ok::<_, String>("ready") })
        })
        .await
        .unwrap();
        assert_eq!(ready, "ready");
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    #[ignore = "explicit synthetic network smoke"]
    async fn live_services_translate_synthetic_text() {
        let input = "Nami synthetic test.\n\nHello & <world>\r\n";
        let targets = vec!["zh-CN".to_string(), "ja".to_string()];
        for engine in ["edge", "google"] {
            let result = translate(engine, input, "auto", &targets, "", false)
                .await
                .unwrap_or_else(|error| panic!("{engine} synthetic smoke failed: {error}"));
            assert_eq!(result.translations.len(), targets.len());
            assert!(!result.detections.is_empty());
            for output in result.translations {
                let lines = split_lines(&output);
                assert_eq!(lines.texts.len(), 4);
                assert_eq!(lines.endings, ["\n", "\n", "\r\n", ""]);
                assert!(lines.texts[1].is_empty() && lines.texts[3].is_empty());
                assert!(!lines.texts[0].trim().is_empty());
                let special = lines.texts[2];
                assert!(!special.trim().is_empty());
                assert!(special.contains('<') && special.contains('>'));
                assert!(!special.contains("&amp;") && !special.contains("&lt;"));
            }
            println!("{engine} PASS");
        }
    }
    #[test]
    fn auto_detection_describes_mixed_sources_instead_of_counting_lines() {
        assert_eq!(
            matched_language(&[
                Some("en".into()),
                Some("zh-CN".into()),
                Some("zh-CN".into())
            ]),
            Some("mul".into())
        );
        assert_eq!(matched_language(&[Some("th".into())]), Some("th".into()));
    }

    #[test]
    fn provider_catalogs_accept_additional_languages_and_reject_other_engines_targets() {
        assert!(validate_options("google", "auto", &["eo".into()]).is_ok());
        assert!(validate_options("edge", "sw", &["en".into()]).is_ok());
        assert!(validate_options("edge", "auto", &["eo".into()]).is_err());
    }

    #[test]
    fn frontend_catalog_matches_each_backend_provider() {
        for (engine, data) in [
            (
                "google",
                include_str!("../../src/shared/languages/google.json"),
            ),
            ("edge", include_str!("../../src/shared/languages/edge.json")),
        ] {
            let frontend: serde_json::Value = serde_json::from_str(data).unwrap();
            let backend = serde_json::to_value(languages(engine)).unwrap();
            assert_eq!(backend, frontend, "{engine} catalogs diverged");
        }
        let frontend_version = include_str!("../../src/translation/history.ts");
        assert!(frontend_version.contains(&format!(
            "TRANSLATION_STRATEGY_VERSION = {STRATEGY_VERSION}"
        )));
    }

    #[test]
    fn oversize_prepared_units_never_make_an_empty_batch() {
        let expanded = "&amp;".repeat(5000);
        assert_eq!(batches(&[&expanded, "later"]), vec![vec![0], vec![1]]);
    }

    #[test]
    fn reuse_requires_current_engine_request_mode_and_strategy() {
        let mut record: crate::model::HistoryRecord = serde_json::from_value(serde_json::json!({
            "id":"synthetic","text":"text","sourceLanguage":"en","targetLanguages":["en","zh-CN"],
            "translations":["text","文本"],"date":"2026-11-17T00:00:00Z","isFavorite":false,"isPinned":false,
            "translation":{"requestedSource":"auto","engine":"edge","strategyVersion":1,"detections":[]}
        })).unwrap();
        assert_eq!(
            reusable_results(
                "edge",
                "text",
                "auto",
                &record.target_languages,
                Some(&record),
                Some(1)
            ),
            [Some("text".into()), None]
        );
        assert_eq!(
            reusable_results(
                "google",
                "text",
                "auto",
                &record.target_languages,
                Some(&record),
                Some(1)
            ),
            [None, None]
        );
        assert_eq!(
            reusable_results(
                "edge",
                "text",
                "en",
                &record.target_languages,
                Some(&record),
                Some(1)
            ),
            [None, None]
        );
        assert_eq!(
            reusable_results(
                "edge",
                "text",
                "auto",
                &record.target_languages,
                Some(&record),
                Some(99)
            ),
            [None, None]
        );
        record.translation.as_mut().unwrap().strategy_version = 0;
        assert_eq!(
            reusable_results(
                "edge",
                "text",
                "auto",
                &record.target_languages,
                Some(&record),
                Some(1)
            ),
            [None, None]
        );
        record.translation = None;
        assert_eq!(
            reusable_results(
                "edge",
                "text",
                "en",
                &record.target_languages,
                Some(&record),
                Some(1)
            ),
            [None, None]
        );
    }

    #[tokio::test]
    #[ignore = "explicit synthetic network smoke"]
    async fn live_services_mixed_text_and_protected_code() {
        let input = "今天的会议取消了。Please send the report tomorrow.\r\n请执行 `npm run build` 并访问 https://example.com/test?a=1&b=2 。";
        for engine in ["google", "edge"] {
            let result = translate(
                engine,
                input,
                "auto",
                &["en".into(), "zh-CN".into()],
                "",
                false,
            )
            .await
            .unwrap_or_else(|error| panic!("{engine}: {error}"));
            assert_eq!(result.source_language, "mul");
            assert!(result.detections.iter().any(|d| d.language == "en"));
            assert!(result.detections.iter().any(|d| d.language == "zh-CN"));
            for translated in &result.translations {
                assert!(
                    translated.contains("`npm run build`"),
                    "{engine}: command changed"
                );
                assert!(
                    translated.contains("https://example.com/test?a=1&b=2"),
                    "{engine}: URL changed"
                );
                assert_eq!(translated.matches("\r\n").count(), 1);
                assert!(!translated.contains("<span") && !translated.contains("nami-code-"));
            }
            assert!(
                !result.translations[0].contains("今天的会议取消了"),
                "{engine}: Chinese sentence left in English result"
            );
            assert!(
                !result.translations[1].contains("Please send the report tomorrow"),
                "{engine}: English sentence left in Chinese result"
            );
        }
    }

    #[test]
    fn dense_sentences_keep_the_original_line_request_budget() {
        let line = "Hi. ".repeat(1000);
        let input = std::iter::repeat_n(line.as_str(), 20)
            .collect::<Vec<_>>()
            .join("\n");
        validate_text(&input).unwrap();
        let prepared = text::units(&input, true);
        assert_eq!(prepared.iter().filter(|unit| unit.translate).count(), 20);
        assert_eq!(
            prepared
                .iter()
                .map(|unit| unit.original)
                .collect::<String>(),
            input
        );
    }
    #[tokio::test]
    async fn empty_and_literal_inputs_do_not_initialize_the_network_client() {
        for engine in ["google", "edge"] {
            for input in [
                "",
                " \t\r\n",
                "12345",
                "`npm run build`",
                "https://example.com",
                "```rust\nlet x = 1;\n```\n",
            ] {
                let result = translate(
                    engine,
                    input,
                    "auto",
                    &["en".into()],
                    "not-a-valid-proxy",
                    true,
                )
                .await
                .unwrap();
                assert_eq!(result.translations, [input]);
                assert_eq!(result.source_language, "und");
                assert!(result.detections.is_empty());
            }
        }
    }

    #[tokio::test]
    #[ignore = "explicit synthetic network smoke"]
    async fn live_services_technical_sentence_protects_bare_commands() {
        let input = "请检查 useEffect 的 dependency array，然后执行 npm run build。";
        for engine in ["google", "edge"] {
            let result = translate(engine, input, "auto", &["en".into()], "", false)
                .await
                .unwrap();
            assert!(
                result.translations[0].contains("npm run build"),
                "{engine}: command changed"
            );
            assert!(
                !result.translations[0].contains("请检查")
                    && !result.translations[0].contains("然后执行"),
                "{engine}: untranslated prose"
            );
        }
    }
    #[test]
    fn merges_only_matching_nonzero_detections_for_sentence_context() {
        let input = "  She bought a bicycle. She rides it to school.  \r\n中文。English.";
        let units = text::units(input, true);
        let detections = units
            .iter()
            .map(|unit| {
                unit.translate.then(|| crate::model::LanguageDetection {
                    raw_language: if unit.original.contains("中文") {
                        "zh-Hans"
                    } else {
                        "en"
                    }
                    .into(),
                    language: if unit.original.contains("中文") {
                        "zh-CN"
                    } else {
                        "en"
                    }
                    .into(),
                    score: Some(1.0),
                })
            })
            .collect::<Vec<_>>();
        let context = context_groups(input, &units, &detections);
        assert_eq!(context.len(), 1);
        assert_eq!(
            context[0].0.original,
            "She bought a bicycle. She rides it to school."
        );
        let mut unknown = detections;
        for detection in unknown.iter_mut().flatten() {
            detection.score = Some(0.0);
        }
        assert!(context_groups(input, &units, &unknown).is_empty());
    }

    #[tokio::test]
    #[ignore = "explicit synthetic network smoke"]
    async fn live_google_preserves_monolingual_sentence_context() {
        let result = translate(
            "google",
            "田中さんは新しい自転車を買いました。それで毎日学校に通っています。",
            "auto",
            &["en".into()],
            "",
            false,
        )
        .await
        .unwrap();
        assert!(result.translations[0].contains("Tanaka"));
        assert!(
            !result.translations[0].contains("I go"),
            "sentence splitting lost the subject"
        );
        assert_eq!(result.source_language, "ja");
    }
    #[test]
    fn context_replacement_preserves_surrounding_whitespace_and_code_fences() {
        let input = "  First sentence. Second sentence.  \r\n```\nuntouched();\n```\n\tRun `npm run build`. Visit https://example.com.\n";
        let units = text::units(input, true);
        let detections = units
            .iter()
            .map(|unit| {
                unit.translate.then(|| crate::model::LanguageDetection {
                    raw_language: "en".into(),
                    language: "en".into(),
                    score: None,
                })
            })
            .collect::<Vec<_>>();
        let groups = context_groups(input, &units, &detections);
        assert_eq!(groups.len(), 2);
        let mut outputs = units
            .iter()
            .map(|unit| unit.original.to_owned())
            .collect::<Vec<_>>();
        for (unit, range) in groups {
            for index in range.clone() {
                outputs[index].clear();
            }
            outputs[range.start] = unit.restore(&unit.prepared).unwrap();
        }
        assert_eq!(outputs.concat(), input);
    }
}
