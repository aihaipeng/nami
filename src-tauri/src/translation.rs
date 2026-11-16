use reqwest::{Client, Response};
use serde_json::{json, Value};
use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

const MAX_RESPONSE_BYTES: usize = 2 * 1024 * 1024;
const TRANSLATION_RETRY_LIMIT: usize = 3;
const TRANSLATION_RETRY_DELAY: Duration = Duration::from_millis(300);
type BatchTranslation = (Vec<String>, Vec<Option<String>>);
type BatchFuture<'a> = Pin<Box<dyn Future<Output = Result<BatchTranslation, String>> + Send + 'a>>;
const LANGUAGES: &[&str] = &[
    "ar", "zh-CN", "zh-TW", "nl", "en", "fr", "de", "id", "it", "ja", "ko", "pl", "pt", "ru", "es",
    "tr", "vi",
];

struct Lines<'a> {
    texts: Vec<&'a str>,
    endings: Vec<&'a str>,
}

fn split_lines(text: &str) -> Lines<'_> {
    let bytes = text.as_bytes();
    let mut texts = Vec::new();
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
            endings.push(&text[end..index]);
            start = index;
        } else {
            index += 1;
        }
    }
    texts.push(&text[start..]);
    endings.push("");
    Lines { texts, endings }
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

pub fn validate_options(engine: &str, source: &str, targets: &[String]) -> Result<(), String> {
    if !matches!(engine, "edge" | "google") {
        return Err("Choose a supported translation engine.".into());
    }
    if source != "auto" && !LANGUAGES.contains(&source) {
        return Err("Choose a supported source language.".into());
    }
    if !(1..=2).contains(&targets.len())
        || targets
            .iter()
            .any(|target| !LANGUAGES.contains(&target.as_str()))
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
                .filter(|_| refresh_target.is_some_and(|changed| index != changed))
                .filter(|record| record.text == text && record.source_language == source)
                .filter(|record| record.target_languages.get(index) == Some(target))
                .and_then(|record| record.translations.get(index))
                .cloned()
        })
        .collect()
}

fn language_code<'a>(engine: &str, code: &'a str) -> &'a str {
    match (engine, code.to_ascii_lowercase().as_str()) {
        ("edge", "auto") => "",
        ("edge", "zh" | "zh-cn" | "zh-hans") => "zh-Hans",
        ("edge", "zh-tw" | "zh-hant") => "zh-Hant",
        (_, "zh" | "zh-cn" | "zh-hans") => "zh-CN",
        (_, "zh-tw" | "zh-hant") => "zh-TW",
        _ => code,
    }
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
        if batch.len() == 64 || units + next > 12_000 {
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

fn escape_html(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn catalog_language(code: &str) -> Option<String> {
    let lower = code.trim().to_ascii_lowercase();
    let mapped = match lower.as_str() {
        "zh" | "zh-cn" | "zh-hans" | "zh-chs" => "zh-CN",
        "zh-tw" | "zh-hant" | "zh-hk" | "zh-mo" | "zh-cht" => "zh-TW",
        "pt-br" | "pt-pt" => "pt",
        other => other,
    };
    LANGUAGES
        .iter()
        .find(|language| language.eq_ignore_ascii_case(mapped))
        .copied()
        .or_else(|| {
            let primary = mapped.split('-').next()?;
            if primary == "zh" {
                return None;
            }
            LANGUAGES
                .iter()
                .find(|language| language.eq_ignore_ascii_case(primary))
                .copied()
        })
        .map(str::to_string)
}

fn matched_language(codes: &[Option<String>]) -> Option<String> {
    let mut counts: Vec<(String, usize)> = Vec::new();
    let mut best: Option<(String, usize)> = None;
    for code in codes.iter().flatten() {
        let count = if let Some(entry) = counts.iter_mut().find(|(name, _)| name == code) {
            entry.1 += 1;
            entry.1
        } else {
            counts.push((code.clone(), 1));
            1
        };
        if best
            .as_ref()
            .is_none_or(|(_, best_count)| count > *best_count)
        {
            best = Some((code.clone(), count));
        }
    }
    best.map(|(code, _)| code)
}

fn parse_response(
    engine: &str,
    data: &[u8],
    count: usize,
    source: &str,
) -> Result<(Vec<String>, Vec<Option<String>>), String> {
    let malformed = || "The translation service returned an invalid response.".to_string();
    let root: Value = serde_json::from_slice(data).map_err(|_| malformed())?;
    if engine == "edge" {
        let rows = root
            .as_array()
            .filter(|rows| rows.len() == count)
            .ok_or_else(malformed)?;
        rows.iter()
            .map(|row| {
                let detected = if source == "auto" {
                    Some(
                        row["detectedLanguage"]["language"]
                            .as_str()
                            .map(catalog_language)
                            .ok_or_else(malformed)?,
                    )
                } else {
                    None
                };
                let text = row["translations"]
                    .as_array()
                    .and_then(|translations| translations.first())
                    .and_then(|translation| translation["text"].as_str())
                    .map(String::from)
                    .ok_or_else(malformed)?;
                Ok((text, detected.flatten()))
            })
            .collect::<Result<Vec<_>, _>>()
            .map(|rows| {
                let (texts, detected) = rows.into_iter().unzip();
                (
                    texts,
                    if source == "auto" {
                        detected
                    } else {
                        Vec::new()
                    },
                )
            })
    } else {
        let root = root.as_array().ok_or_else(malformed)?;
        let texts = root
            .first()
            .and_then(Value::as_array)
            .filter(|texts| texts.len() == count)
            .ok_or_else(malformed)?;
        let detected = if source == "auto" {
            let detected = root
                .get(1)
                .and_then(Value::as_array)
                .filter(|codes| codes.len() == count)
                .ok_or_else(malformed)?;
            detected
                .iter()
                .map(|code| code.as_str().map(catalog_language).ok_or_else(malformed))
                .collect::<Result<Vec<_>, _>>()?
        } else {
            Vec::new()
        };
        let texts = texts
            .iter()
            .map(|text| {
                text.as_str()
                    .map(|text| html_escape::decode_html_entities(text).into_owned())
                    .ok_or_else(malformed)
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok((texts, detected))
    }
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

fn join_lines(translations: &[String], endings: &[&str]) -> String {
    let mut result = String::new();
    for (translation, ending) in translations.iter().zip(endings) {
        result.push_str(translation);
        result.push_str(ending);
    }
    result
}

fn request(
    client: &Client,
    engine: &str,
    active: &[&str],
    source: &str,
    target: &str,
) -> reqwest::RequestBuilder {
    if engine == "edge" {
        client
            .post("https://edge.microsoft.com/translate/translatetext")
            .query(&[
                ("from", language_code(engine, source)),
                ("to", language_code(engine, target)),
                ("isEnterpriseClient", "false"),
            ])
            .json(&active)
    } else {
        let escaped: Vec<String> = active.iter().map(|text| escape_html(text)).collect();
        client
            .post("https://translate-pa.googleapis.com/v1/translateHtml")
            .header("Content-Type", "application/json+protobuf")
            .json(&json!([
                [
                    escaped,
                    language_code(engine, source),
                    language_code(engine, target)
                ],
                "te_lib"
            ]))
            // Public Translate Element frontend key used by the reference client.
            .header("X-Goog-Api-Key", "AIzaSyATBXajvzQLTDHEQbcpq0Ihe0vWDHmO520")
    }
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
        let response = request(client, engine, active, source, target)
            .send()
            .await
            .map_err(network_error)?;
        let data = response_bytes(response).await?;
        parse_response(engine, &data, active.len(), source)
    })
}

pub async fn translate(
    engine: &str,
    text: &str,
    source: &str,
    targets: &[String],
) -> Result<(Vec<String>, String), String> {
    validate_text(text)?;
    validate_options(engine, source, targets)?;
    let lines = split_lines(text);
    let batches = batches(&lines.texts);
    if batches.is_empty() {
        return Ok((
            targets.iter().map(|_| text.to_string()).collect(),
            source.to_string(),
        ));
    }
    let client = Client::builder()
        .user_agent("Nami/0.1")
        .timeout(Duration::from_secs(20))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|_| "Unable to initialize the translation service.".to_string())?;
    let mut results = Vec::with_capacity(targets.len());
    let mut detected = Vec::new();
    for (target_index, target) in targets.iter().enumerate() {
        let mut translations: Vec<String> =
            lines.texts.iter().map(|text| text.to_string()).collect();
        for batch in &batches {
            let active: Vec<&str> = batch.iter().map(|index| lines.texts[*index]).collect();
            let (translated, languages) = with_translation_retries(TRANSLATION_RETRY_DELAY, || {
                fetch_batch(&client, engine, &active, source, target)
            })
            .await?;
            if target_index == 0 && source == "auto" {
                detected.extend(languages);
            }
            for (index, translated) in batch.iter().zip(translated) {
                translations[*index] = translated;
            }
        }
        results.push(join_lines(&translations, &lines.endings));
    }
    let source_language = if source == "auto" {
        matched_language(&detected).unwrap_or_else(|| source.to_string())
    } else {
        source.to_string()
    };
    Ok((results, source_language))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn language_switch_reuses_only_unchanged_columns() {
        let record = crate::model::HistoryRecord {
            id: "previous".into(),
            text: "Synthetic".into(),
            source_language: "auto".into(),
            target_languages: vec!["en".into(), "zh-CN".into()],
            translations: vec!["English".into(), "Chinese".into()],
            date: "test".into(),
            is_favorite: false,
            is_pinned: false,
            source_app: None,
        };
        assert_eq!(
            reusable_results(
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
            reusable_results("Synthetic", "auto", &targets, Some(&record), Some(0)),
            vec![None, Some("Chinese".into())]
        );
        assert_eq!(
            reusable_results("Synthetic", "de", &targets, Some(&record), Some(0)),
            vec![None, None]
        );
        assert_eq!(
            reusable_results("Edited", "auto", &targets, Some(&record), Some(0)),
            vec![None, None]
        );
        assert_eq!(
            reusable_results("Synthetic", "auto", &targets, None, Some(0)),
            vec![None, None]
        );
        assert_eq!(
            reusable_results("Synthetic", "auto", &["en".into()], Some(&record), Some(1)),
            vec![Some("English".into())]
        );
        assert_eq!(
            reusable_results(
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
    fn builds_fixed_provider_requests_with_one_content_type() {
        let client = Client::builder().user_agent("Nami/0.1").build().unwrap();
        let active = ["A & B <world>", "Second line"];
        let edge = request(&client, "edge", &active, "auto", "zh-CN")
            .build()
            .unwrap();
        assert_eq!(edge.method(), reqwest::Method::POST);
        assert_eq!(edge.url().scheme(), "https");
        assert_eq!(edge.url().host_str(), Some("edge.microsoft.com"));
        assert_eq!(edge.url().path(), "/translate/translatetext");
        assert_eq!(
            edge.url().as_str(),
            "https://edge.microsoft.com/translate/translatetext?from=&to=zh-Hans&isEnterpriseClient=false"
        );
        assert_eq!(edge.headers().get_all("Content-Type").iter().count(), 1);
        assert_eq!(edge.headers()["Content-Type"], "application/json");
        let edge_body: Value =
            serde_json::from_slice(edge.body().unwrap().as_bytes().unwrap()).unwrap();
        assert_eq!(edge_body, json!(active));

        let google = request(&client, "google", &active, "zh-Hans", "zh-Hant")
            .build()
            .unwrap();
        assert_eq!(google.method(), reqwest::Method::POST);
        assert_eq!(
            google.url().as_str(),
            "https://translate-pa.googleapis.com/v1/translateHtml"
        );
        assert_eq!(google.headers().get_all("Content-Type").iter().count(), 1);
        assert_eq!(
            google.headers()["Content-Type"],
            "application/json+protobuf"
        );
        assert!(google.headers().contains_key("X-Goog-Api-Key"));
        let google_body: Value =
            serde_json::from_slice(google.body().unwrap().as_bytes().unwrap()).unwrap();
        assert_eq!(
            google_body,
            json!([
                [["A &amp; B &lt;world&gt;", "Second line"], "zh-CN", "zh-TW"],
                "te_lib"
            ])
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
    fn maps_chinese_aliases_per_provider() {
        assert_eq!(language_code("edge", "auto"), "");
        for code in ["zh", "ZH-cn", "zh-Hans"] {
            assert_eq!(language_code("edge", code), "zh-Hans");
            assert_eq!(language_code("google", code), "zh-CN");
        }
        for code in ["zh-TW", "zh-Hant"] {
            assert_eq!(language_code("edge", code), "zh-Hant");
            assert_eq!(language_code("google", code), "zh-TW");
        }
        assert_eq!(language_code("google", "auto"), "auto");
    }

    #[test]
    fn parses_strict_provider_responses_and_html_once() {
        let edge = br#"[{"detectedLanguage":{"language":"en"},"translations":[{"text":"Hello"}]}]"#;
        let (texts, detected) = parse_response("edge", edge, 1, "auto").unwrap();
        assert_eq!(texts, ["Hello"]);
        assert_eq!(detected, [Some("en".to_string())]);
        assert!(parse_response("edge", edge, 2, "auto").is_err());
        assert!(
            parse_response("edge", br#"[{"translations":[{"text":"x"}]}]"#, 1, "auto").is_err()
        );
        assert!(parse_response("edge", br#"[{"translations":[{"text":2}]}]"#, 1, "en").is_err());
        assert_eq!(escape_html("A & B < C > D"), "A &amp; B &lt; C &gt; D");
        let google = br#"[["&lt;b&gt;&amp; &#39; &#x1F600; &amp;lt;"],["en"]]"#;
        let (texts, detected) = parse_response("google", google, 1, "auto").unwrap();
        assert_eq!(texts, ["<b>& ' 😀 &lt;"]);
        assert_eq!(detected, [Some("en".to_string())]);
        assert!(parse_response("google", google, 2, "auto").is_err());
        assert!(parse_response("google", br#"[["x"],["en","ja"]]"#, 1, "auto").is_err());
        assert!(parse_response("google", br#"[[2]]"#, 1, "en").is_err());
        assert!(parse_response("google", b"invalid", 1, "en").is_err());
    }

    #[test]
    fn auto_detection_uses_the_most_common_catalog_language() {
        let edge = br#"[{"detectedLanguage":{"language":"zh-Hans"},"translations":[{"text":"A"}]},{"detectedLanguage":{"language":"en-US"},"translations":[{"text":"B"}]},{"detectedLanguage":{"language":"en"},"translations":[{"text":"C"}]},{"detectedLanguage":{"language":"la"},"translations":[{"text":"D"}]}]"#;
        let (texts, detected) = parse_response("edge", edge, 4, "auto").unwrap();
        assert_eq!(texts, ["A", "B", "C", "D"]);
        assert_eq!(
            detected,
            [
                Some("zh-CN".to_string()),
                Some("en".to_string()),
                Some("en".to_string()),
                None
            ]
        );
        assert_eq!(matched_language(&detected), Some("en".to_string()));
        assert_eq!(
            matched_language(&[Some("ja".into()), Some("fr".into())]),
            Some("ja".to_string())
        );
        let google = r#"[["甲","乙"],["zh-Hans","zh-Hant"]]"#.as_bytes();
        assert_eq!(
            parse_response("google", google, 2, "auto").unwrap().1,
            [Some("zh-CN".to_string()), Some("zh-TW".to_string())]
        );
        let (texts, detected) = parse_response("edge", edge, 4, "fr").unwrap();
        assert_eq!(texts, ["A", "B", "C", "D"]);
        assert!(detected.is_empty());
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
            let (results, detected) = translate(engine, input, "auto", &targets)
                .await
                .unwrap_or_else(|error| panic!("{engine} synthetic smoke failed: {error}"));
            assert_eq!(results.len(), targets.len());
            assert!(LANGUAGES.contains(&detected.as_str()));
            for output in results {
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
}
