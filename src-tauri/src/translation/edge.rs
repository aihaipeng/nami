use reqwest::{Client, RequestBuilder};
use serde_json::Value;

use super::{Language, SegmentResult};
use crate::model::LanguageDetection;

// Edge accepted all source/target codes in the 2026-11-17 synthetic protocol probe.
const LANGUAGES: &[Language] = &[
    Language {
        code: "af",
        name: "Afrikaans",
        source: true,
        target: true,
    },
    Language {
        code: "sq",
        name: "Albanian",
        source: true,
        target: true,
    },
    Language {
        code: "am",
        name: "Amharic",
        source: true,
        target: true,
    },
    Language {
        code: "ar",
        name: "Arabic",
        source: true,
        target: true,
    },
    Language {
        code: "hy",
        name: "Armenian",
        source: true,
        target: true,
    },
    Language {
        code: "as",
        name: "Assamese",
        source: true,
        target: true,
    },
    Language {
        code: "awa",
        name: "Awadhi",
        source: true,
        target: true,
    },
    Language {
        code: "az",
        name: "Azerbaijani",
        source: true,
        target: true,
    },
    Language {
        code: "bn",
        name: "Bangla",
        source: true,
        target: true,
    },
    Language {
        code: "ba",
        name: "Bashkir",
        source: true,
        target: true,
    },
    Language {
        code: "eu",
        name: "Basque",
        source: true,
        target: true,
    },
    Language {
        code: "be",
        name: "Belarusian",
        source: true,
        target: true,
    },
    Language {
        code: "bho",
        name: "Bhojpuri",
        source: true,
        target: true,
    },
    Language {
        code: "brx",
        name: "Bodo",
        source: true,
        target: true,
    },
    Language {
        code: "bs",
        name: "Bosnian",
        source: true,
        target: true,
    },
    Language {
        code: "bg",
        name: "Bulgarian",
        source: true,
        target: true,
    },
    Language {
        code: "yue",
        name: "Cantonese (Traditional)",
        source: true,
        target: true,
    },
    Language {
        code: "ca",
        name: "Catalan",
        source: true,
        target: true,
    },
    Language {
        code: "ceb",
        name: "Cebuano",
        source: true,
        target: true,
    },
    Language {
        code: "ce",
        name: "Chechen",
        source: true,
        target: true,
    },
    Language {
        code: "hne",
        name: "Chhattisgarhi",
        source: true,
        target: true,
    },
    Language {
        code: "lzh",
        name: "Chinese (Literary)",
        source: true,
        target: true,
    },
    Language {
        code: "zh-CN",
        name: "Chinese Simplified",
        source: true,
        target: true,
    },
    Language {
        code: "zh-TW",
        name: "Chinese Traditional",
        source: true,
        target: true,
    },
    Language {
        code: "cv",
        name: "Chuvash",
        source: true,
        target: true,
    },
    Language {
        code: "hr",
        name: "Croatian",
        source: true,
        target: true,
    },
    Language {
        code: "cs",
        name: "Czech",
        source: true,
        target: true,
    },
    Language {
        code: "da",
        name: "Danish",
        source: true,
        target: true,
    },
    Language {
        code: "fa-AF",
        name: "Dari",
        source: true,
        target: true,
    },
    Language {
        code: "luo",
        name: "Dholuo",
        source: true,
        target: true,
    },
    Language {
        code: "dv",
        name: "Divehi",
        source: true,
        target: true,
    },
    Language {
        code: "doi",
        name: "Dogri",
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
        code: "dz",
        name: "Dzongkha",
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
        code: "et",
        name: "Estonian",
        source: true,
        target: true,
    },
    Language {
        code: "fo",
        name: "Faroese",
        source: true,
        target: true,
    },
    Language {
        code: "fj",
        name: "Fijian",
        source: true,
        target: true,
    },
    Language {
        code: "fil",
        name: "Filipino",
        source: true,
        target: true,
    },
    Language {
        code: "fi",
        name: "Finnish",
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
        code: "fr-CA",
        name: "French (Canada)",
        source: true,
        target: true,
    },
    Language {
        code: "gl",
        name: "Galician",
        source: true,
        target: true,
    },
    Language {
        code: "lg",
        name: "Ganda",
        source: true,
        target: true,
    },
    Language {
        code: "ka",
        name: "Georgian",
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
        code: "el",
        name: "Greek",
        source: true,
        target: true,
    },
    Language {
        code: "gu",
        name: "Gujarati",
        source: true,
        target: true,
    },
    Language {
        code: "ht",
        name: "Haitian Creole",
        source: true,
        target: true,
    },
    Language {
        code: "ha",
        name: "Hausa",
        source: true,
        target: true,
    },
    Language {
        code: "haw",
        name: "Hawaiian",
        source: true,
        target: true,
    },
    Language {
        code: "he",
        name: "Hebrew",
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
        code: "hmn",
        name: "Hmong Daw",
        source: true,
        target: true,
    },
    Language {
        code: "hu",
        name: "Hungarian",
        source: true,
        target: true,
    },
    Language {
        code: "is",
        name: "Icelandic",
        source: true,
        target: true,
    },
    Language {
        code: "ig",
        name: "Igbo",
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
        code: "ikt",
        name: "Inuinnaqtun",
        source: true,
        target: true,
    },
    Language {
        code: "iu",
        name: "Inuktitut",
        source: true,
        target: true,
    },
    Language {
        code: "iu-Latn",
        name: "Inuktitut (Latin)",
        source: true,
        target: true,
    },
    Language {
        code: "ga",
        name: "Irish",
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
        code: "kn",
        name: "Kannada",
        source: true,
        target: true,
    },
    Language {
        code: "ks",
        name: "Kashmiri",
        source: true,
        target: true,
    },
    Language {
        code: "kk",
        name: "Kazakh",
        source: true,
        target: true,
    },
    Language {
        code: "kha",
        name: "Khasi",
        source: true,
        target: true,
    },
    Language {
        code: "km",
        name: "Khmer",
        source: true,
        target: true,
    },
    Language {
        code: "rw",
        name: "Kinyarwanda",
        source: true,
        target: true,
    },
    Language {
        code: "tlh",
        name: "Klingon (Latin)",
        source: true,
        target: true,
    },
    Language {
        code: "tlh-Piqd",
        name: "Klingon (pIqaD)",
        source: true,
        target: true,
    },
    Language {
        code: "gom",
        name: "Konkani",
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
        code: "ckb",
        name: "Kurdish (Central)",
        source: true,
        target: true,
    },
    Language {
        code: "ku",
        name: "Kurdish (Northern)",
        source: true,
        target: true,
    },
    Language {
        code: "ky",
        name: "Kyrgyz",
        source: true,
        target: true,
    },
    Language {
        code: "lo",
        name: "Lao",
        source: true,
        target: true,
    },
    Language {
        code: "la",
        name: "Latin",
        source: true,
        target: true,
    },
    Language {
        code: "lv",
        name: "Latvian",
        source: true,
        target: true,
    },
    Language {
        code: "ln",
        name: "Lingala",
        source: true,
        target: true,
    },
    Language {
        code: "lt",
        name: "Lithuanian",
        source: true,
        target: true,
    },
    Language {
        code: "dsb",
        name: "Lower Sorbian",
        source: true,
        target: true,
    },
    Language {
        code: "lb",
        name: "Luxembourgish",
        source: true,
        target: true,
    },
    Language {
        code: "mk",
        name: "Macedonian",
        source: true,
        target: true,
    },
    Language {
        code: "mag",
        name: "Magahi",
        source: true,
        target: true,
    },
    Language {
        code: "mai",
        name: "Maithili",
        source: true,
        target: true,
    },
    Language {
        code: "mg",
        name: "Malagasy",
        source: true,
        target: true,
    },
    Language {
        code: "ms",
        name: "Malay",
        source: true,
        target: true,
    },
    Language {
        code: "ml",
        name: "Malayalam",
        source: true,
        target: true,
    },
    Language {
        code: "mt",
        name: "Maltese",
        source: true,
        target: true,
    },
    Language {
        code: "mni",
        name: "Manipuri",
        source: true,
        target: true,
    },
    Language {
        code: "mr",
        name: "Marathi",
        source: true,
        target: true,
    },
    Language {
        code: "lus",
        name: "Mizo",
        source: true,
        target: true,
    },
    Language {
        code: "mn",
        name: "Mongolian (Cyrillic)",
        source: true,
        target: true,
    },
    Language {
        code: "mn-Mong",
        name: "Mongolian (Traditional)",
        source: true,
        target: true,
    },
    Language {
        code: "my",
        name: "Myanmar (Burmese)",
        source: true,
        target: true,
    },
    Language {
        code: "mi",
        name: "Māori",
        source: true,
        target: true,
    },
    Language {
        code: "ne",
        name: "Nepali",
        source: true,
        target: true,
    },
    Language {
        code: "nb",
        name: "Norwegian",
        source: true,
        target: true,
    },
    Language {
        code: "ny",
        name: "Nyanja",
        source: true,
        target: true,
    },
    Language {
        code: "oc",
        name: "Occitan",
        source: true,
        target: true,
    },
    Language {
        code: "or",
        name: "Odia",
        source: true,
        target: true,
    },
    Language {
        code: "ps",
        name: "Pashto",
        source: true,
        target: true,
    },
    Language {
        code: "fa",
        name: "Persian",
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
        code: "pt-PT",
        name: "Portuguese (Portugal)",
        source: true,
        target: true,
    },
    Language {
        code: "pa",
        name: "Punjabi",
        source: true,
        target: true,
    },
    Language {
        code: "otq",
        name: "Querétaro Otomi",
        source: true,
        target: true,
    },
    Language {
        code: "ro",
        name: "Romanian",
        source: true,
        target: true,
    },
    Language {
        code: "rn",
        name: "Rundi",
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
        code: "sm",
        name: "Samoan",
        source: true,
        target: true,
    },
    Language {
        code: "sa",
        name: "Sanskrit",
        source: true,
        target: true,
    },
    Language {
        code: "sat",
        name: "Santali",
        source: true,
        target: true,
    },
    Language {
        code: "sr",
        name: "Serbian (Cyrillic)",
        source: true,
        target: true,
    },
    Language {
        code: "sr-Latn",
        name: "Serbian (Latin)",
        source: true,
        target: true,
    },
    Language {
        code: "st",
        name: "Sesotho",
        source: true,
        target: true,
    },
    Language {
        code: "nso",
        name: "Sesotho sa Leboa",
        source: true,
        target: true,
    },
    Language {
        code: "tn",
        name: "Setswana",
        source: true,
        target: true,
    },
    Language {
        code: "sn",
        name: "Shona",
        source: true,
        target: true,
    },
    Language {
        code: "sd",
        name: "Sindhi",
        source: true,
        target: true,
    },
    Language {
        code: "si",
        name: "Sinhala",
        source: true,
        target: true,
    },
    Language {
        code: "sk",
        name: "Slovak",
        source: true,
        target: true,
    },
    Language {
        code: "sl",
        name: "Slovenian",
        source: true,
        target: true,
    },
    Language {
        code: "so",
        name: "Somali",
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
        code: "es-MX",
        name: "Spanish (Mexico)",
        source: true,
        target: true,
    },
    Language {
        code: "su",
        name: "Sundanese",
        source: true,
        target: true,
    },
    Language {
        code: "sw",
        name: "Swahili",
        source: true,
        target: true,
    },
    Language {
        code: "sv",
        name: "Swedish",
        source: true,
        target: true,
    },
    Language {
        code: "ty",
        name: "Tahitian",
        source: true,
        target: true,
    },
    Language {
        code: "tg",
        name: "Tajik",
        source: true,
        target: true,
    },
    Language {
        code: "ta",
        name: "Tamil",
        source: true,
        target: true,
    },
    Language {
        code: "tt",
        name: "Tatar",
        source: true,
        target: true,
    },
    Language {
        code: "te",
        name: "Telugu",
        source: true,
        target: true,
    },
    Language {
        code: "tet",
        name: "Tetum",
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
        code: "bo",
        name: "Tibetan",
        source: true,
        target: true,
    },
    Language {
        code: "ti",
        name: "Tigrinya",
        source: true,
        target: true,
    },
    Language {
        code: "to",
        name: "Tongan",
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
        code: "tk",
        name: "Turkmen",
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
        code: "hsb",
        name: "Upper Sorbian",
        source: true,
        target: true,
    },
    Language {
        code: "ur",
        name: "Urdu",
        source: true,
        target: true,
    },
    Language {
        code: "ug",
        name: "Uyghur",
        source: true,
        target: true,
    },
    Language {
        code: "uz",
        name: "Uzbek (Latin)",
        source: true,
        target: true,
    },
    Language {
        code: "vi",
        name: "Vietnamese",
        source: true,
        target: true,
    },
    Language {
        code: "cy",
        name: "Welsh",
        source: true,
        target: true,
    },
    Language {
        code: "xh",
        name: "Xhosa",
        source: true,
        target: true,
    },
    Language {
        code: "yo",
        name: "Yoruba",
        source: true,
        target: true,
    },
    Language {
        code: "yua",
        name: "Yucatec Maya",
        source: true,
        target: true,
    },
    Language {
        code: "zu",
        name: "Zulu",
        source: true,
        target: true,
    },
    Language {
        code: "jv",
        name: "basa Djawa",
        source: true,
        target: true,
    },
];

pub fn languages() -> &'static [Language] {
    LANGUAGES
}

pub(super) fn language_code(code: &str) -> &str {
    match code {
        "auto" => "",
        "zh-CN" => "zh-Hans",
        "zh-TW" => "zh-Hant",
        "ckb" => "ku",
        "ku" => "kmr",
        "hmn" => "mww",
        "mn" => "mn-Cyrl",
        "sr" => "sr-Cyrl",
        "fa-AF" => "prs",
        "lg" => "lug",
        "ny" => "nya",
        "rn" => "run",
        "tlh" => "tlh-Latn",
        other => other,
    }
}

pub(super) fn normalize_language(code: &str) -> String {
    let trimmed = code.trim();
    let lower = trimmed.to_ascii_lowercase();
    let canonical = match lower.as_str() {
        "zh" | "zh-hans" | "zh-cn" | "zh-chs" => "zh-CN",
        "zh-hant" | "zh-tw" | "zh-hk" | "zh-mo" | "zh-cht" => "zh-TW",
        "pt-br" => "pt",
        "ku" => "ckb",
        "kmr" => "ku",
        "mww" => "hmn",
        "mn-cyrl" => "mn",
        "sr-cyrl" => "sr",
        "prs" => "fa-AF",
        "lug" => "lg",
        "nya" => "ny",
        "run" => "rn",
        "tlh-latn" => "tlh",
        "iu-cans" => "iu",
        "az-az" => "az",
        "iw" => "he",
        "tl" => "fil",
        "no" => "nb",
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

pub(super) fn request(
    client: &Client,
    texts: &[&str],
    source: &str,
    target: &str,
) -> RequestBuilder {
    client
        .post("https://edge.microsoft.com/translate/translatetext")
        .query(&[
            ("from", language_code(source)),
            ("to", language_code(target)),
            ("isEnterpriseClient", "false"),
        ])
        .json(&texts)
}

pub(super) fn parse(data: &[u8], count: usize, source: &str) -> Result<Vec<SegmentResult>, String> {
    let malformed = || "The translation service returned an invalid response.".to_string();
    let root: Value = serde_json::from_slice(data).map_err(|_| malformed())?;
    let rows = root
        .as_array()
        .filter(|rows| rows.len() == count)
        .ok_or_else(malformed)?;
    rows.iter()
        .map(|row| {
            let detection = if source == "auto" {
                let detected = &row["detectedLanguage"];
                let raw = detected["language"]
                    .as_str()
                    .filter(|code| {
                        !code.is_empty()
                            && code.len() <= 64
                            && code
                                .bytes()
                                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
                    })
                    .ok_or_else(malformed)?;
                let score = match detected.get("score") {
                    None | Some(Value::Null) => None,
                    Some(value) => Some(
                        value
                            .as_f64()
                            .filter(|score| (0.0..=1.0).contains(score))
                            .ok_or_else(malformed)?,
                    ),
                };
                Some(LanguageDetection {
                    raw_language: raw.to_string(),
                    language: normalize_language(raw),
                    score,
                })
            } else {
                None
            };
            let translations = row["translations"]
                .as_array()
                .filter(|items| items.len() == 1)
                .ok_or_else(malformed)?;
            let text = translations[0]["text"]
                .as_str()
                .ok_or_else(malformed)?
                .to_string();
            Ok(SegmentResult { text, detection })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn parse_value(value: Value, count: usize, source: &str) -> Result<Vec<SegmentResult>, String> {
        parse(&serde_json::to_vec(&value).unwrap(), count, source)
    }

    #[test]
    fn auto_preserves_original_code_and_zero_or_one_scores() {
        let result = parse_value(json!([
            {"detectedLanguage":{"language":"zh-Hant","score":1.0},"translations":[{"text":"A"}]},
            {"detectedLanguage":{"language":"qaa-Zzzz","score":0.0},"translations":[{"text":"B"}]}
        ]), 2, "auto").unwrap();
        assert_eq!(result[0].text, "A");
        let first = result[0].detection.as_ref().unwrap();
        assert_eq!(first.raw_language, "zh-Hant");
        assert_eq!(first.language, "zh-TW");
        assert_eq!(first.score, Some(1.0));
        let second = result[1].detection.as_ref().unwrap();
        assert_eq!(second.raw_language, "qaa-Zzzz");
        assert_eq!(second.language, "qaa-Zzzz");
        assert_eq!(second.score, Some(0.0));
    }

    #[test]
    fn manual_source_does_not_require_detection_or_decode_html() {
        let result = parse_value(
            json!([{"translations":[{"text":"<span translate=\"no\">npm run build</span>&amp;"}]}]),
            1,
            "zh-CN",
        )
        .unwrap();
        assert!(result[0].detection.is_none());
        assert_eq!(
            result[0].text,
            "<span translate=\"no\">npm run build</span>&amp;"
        );
    }

    #[test]
    fn aliases_keep_kurdish_meanings_and_script_variants_separate() {
        for (provider, canonical) in [
            ("zh-Hans", "zh-CN"),
            ("zh-Hant", "zh-TW"),
            ("ku", "ckb"),
            ("kmr", "ku"),
            ("mww", "hmn"),
            ("mn-Cyrl", "mn"),
            ("mn-Mong", "mn-Mong"),
            ("sr-Cyrl", "sr"),
            ("sr-Latn", "sr-Latn"),
            ("prs", "fa-AF"),
            ("lug", "lg"),
            ("nya", "ny"),
            ("run", "rn"),
            ("he", "he"),
            ("fil", "fil"),
            ("nb", "nb"),
        ] {
            assert_eq!(normalize_language(provider), canonical);
            assert_eq!(language_code(canonical), provider);
        }
        assert_eq!(language_code("auto"), "");
        assert_eq!(normalize_language("en-US"), "en");
        assert_eq!(normalize_language("pt-PT"), "pt-PT");
        assert_eq!(normalize_language("pt-BR"), "pt");
    }

    #[test]
    fn rejects_misaligned_or_malformed_translation_responses() {
        for value in [
            json!({"error":{"code":400}}),
            json!([]),
            json!([{"detectedLanguage":{"language":"en"},"translations":[]}]),
            json!([{"detectedLanguage":{"language":"en"},"translations":[{"text":5}]}]),
            json!([{"translations":[{"text":"A"}]}]),
            json!([{"detectedLanguage":{"language":""},"translations":[{"text":"A"}]}]),
            json!([{"detectedLanguage":{"language":"en","score":"1"},"translations":[{"text":"A"}]}]),
            json!([{"detectedLanguage":{"language":"en","score":-0.1},"translations":[{"text":"A"}]}]),
            json!([{"detectedLanguage":{"language":"en","score":1.1},"translations":[{"text":"A"}]}]),
        ] {
            assert!(parse_value(value, 1, "auto").is_err());
        }
        assert!(parse(b"not-json", 1, "auto").is_err());
    }

    #[test]
    fn absent_confidence_remains_unknown_instead_of_becoming_zero() {
        let result = parse_value(
            json!([{"detectedLanguage":{"language":"en"},"translations":[{"text":"A"}]}]),
            1,
            "auto",
        )
        .unwrap();
        assert_eq!(result[0].detection.as_ref().unwrap().score, None);
    }

    #[test]
    fn request_maps_only_provider_codes_and_keeps_prepared_text() {
        let request = request(
            &Client::new(),
            &["<span translate=\"no\">npm run build</span>"],
            "auto",
            "zh-CN",
        )
        .build()
        .unwrap();
        assert_eq!(request.url().host_str(), Some("edge.microsoft.com"));
        assert!(request
            .url()
            .query_pairs()
            .any(|(key, value)| key == "from" && value.is_empty()));
        assert!(request
            .url()
            .query_pairs()
            .any(|(key, value)| key == "to" && value == "zh-Hans"));
        let body: Value =
            serde_json::from_slice(request.body().unwrap().as_bytes().unwrap()).unwrap();
        assert_eq!(body, json!(["<span translate=\"no\">npm run build</span>"]));
    }

    #[test]
    fn catalog_contains_unique_round_trippable_capabilities() {
        assert!(languages().len() >= 150);
        let mut codes = std::collections::HashSet::new();
        for language in languages() {
            assert!(codes.insert(language.code));
            assert!(language.source || language.target);
            assert!(!language.name.is_empty());
            assert_eq!(
                normalize_language(language_code(language.code)),
                language.code
            );
        }
        assert!(languages()
            .iter()
            .any(|item| item.code == "sw" && item.source && item.target));
        assert!(languages()
            .iter()
            .any(|item| item.code == "lzh" && item.source && item.target));
    }
}
