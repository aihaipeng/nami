use super::{Language, SegmentResult};
use crate::model::LanguageDetection;
use reqwest::{Client, RequestBuilder};
use serde_json::{json, Value};

// Google Translate frontend catalog, checked 2026-11-17; zh-TW is also accepted as an explicit source.
#[rustfmt::skip]
const LANGUAGES: &[Language] = &[
    Language { code: "ab", name: "Abkhaz", source: true, target: true },
    Language { code: "ace", name: "Acehnese", source: true, target: true },
    Language { code: "ach", name: "Acholi", source: true, target: true },
    Language { code: "aa", name: "Afar", source: true, target: true },
    Language { code: "af", name: "Afrikaans", source: true, target: true },
    Language { code: "sq", name: "Albanian", source: true, target: true },
    Language { code: "alz", name: "Alur", source: true, target: true },
    Language { code: "am", name: "Amharic", source: true, target: true },
    Language { code: "ar", name: "Arabic", source: true, target: true },
    Language { code: "hy", name: "Armenian", source: true, target: true },
    Language { code: "as", name: "Assamese", source: true, target: true },
    Language { code: "av", name: "Avar", source: true, target: true },
    Language { code: "awa", name: "Awadhi", source: true, target: true },
    Language { code: "ay", name: "Aymara", source: true, target: true },
    Language { code: "az", name: "Azerbaijani", source: true, target: true },
    Language { code: "ban", name: "Balinese", source: true, target: true },
    Language { code: "bal", name: "Baluchi", source: true, target: true },
    Language { code: "bm", name: "Bambara", source: true, target: true },
    Language { code: "bci", name: "Baoulé", source: true, target: true },
    Language { code: "ba", name: "Bashkir", source: true, target: true },
    Language { code: "eu", name: "Basque", source: true, target: true },
    Language { code: "btx", name: "Batak Karo", source: true, target: true },
    Language { code: "bts", name: "Batak Simalungun", source: true, target: true },
    Language { code: "bbc", name: "Batak Toba", source: true, target: true },
    Language { code: "be", name: "Belarusian", source: true, target: true },
    Language { code: "bem", name: "Bemba", source: true, target: true },
    Language { code: "bn", name: "Bengali", source: true, target: true },
    Language { code: "bew", name: "Betawi", source: true, target: true },
    Language { code: "bho", name: "Bhojpuri", source: true, target: true },
    Language { code: "bik", name: "Bikol", source: true, target: true },
    Language { code: "bs", name: "Bosnian", source: true, target: true },
    Language { code: "br", name: "Breton", source: true, target: true },
    Language { code: "bg", name: "Bulgarian", source: true, target: true },
    Language { code: "bua", name: "Buryat", source: true, target: true },
    Language { code: "yue", name: "Cantonese", source: true, target: true },
    Language { code: "ca", name: "Catalan", source: true, target: true },
    Language { code: "ceb", name: "Cebuano", source: true, target: true },
    Language { code: "ch", name: "Chamorro", source: true, target: true },
    Language { code: "ce", name: "Chechen", source: true, target: true },
    Language { code: "ny", name: "Chichewa", source: true, target: true },
    Language { code: "zh-CN", name: "Chinese (Simplified)", source: true, target: true },
    Language { code: "zh-TW", name: "Chinese (Traditional)", source: true, target: true },
    Language { code: "chk", name: "Chuukese", source: true, target: true },
    Language { code: "cv", name: "Chuvash", source: true, target: true },
    Language { code: "co", name: "Corsican", source: true, target: true },
    Language { code: "crh", name: "Crimean Tatar (Cyrillic)", source: true, target: true },
    Language { code: "crh-Latn", name: "Crimean Tatar (Latin)", source: true, target: true },
    Language { code: "hr", name: "Croatian", source: true, target: true },
    Language { code: "cs", name: "Czech", source: true, target: true },
    Language { code: "da", name: "Danish", source: true, target: true },
    Language { code: "fa-AF", name: "Dari", source: true, target: true },
    Language { code: "dv", name: "Dhivehi", source: true, target: true },
    Language { code: "din", name: "Dinka", source: true, target: true },
    Language { code: "doi", name: "Dogri", source: true, target: true },
    Language { code: "dov", name: "Dombe", source: true, target: true },
    Language { code: "nl", name: "Dutch", source: true, target: true },
    Language { code: "dyu", name: "Dyula", source: true, target: true },
    Language { code: "dz", name: "Dzongkha", source: true, target: true },
    Language { code: "en", name: "English", source: true, target: true },
    Language { code: "eo", name: "Esperanto", source: true, target: true },
    Language { code: "et", name: "Estonian", source: true, target: true },
    Language { code: "ee", name: "Ewe", source: true, target: true },
    Language { code: "fo", name: "Faroese", source: true, target: true },
    Language { code: "fj", name: "Fijian", source: true, target: true },
    Language { code: "fil", name: "Filipino", source: true, target: true },
    Language { code: "fi", name: "Finnish", source: true, target: true },
    Language { code: "fon", name: "Fon", source: true, target: true },
    Language { code: "fr", name: "French", source: true, target: true },
    Language { code: "fr-CA", name: "French (Canada)", source: true, target: true },
    Language { code: "fy", name: "Frisian", source: true, target: true },
    Language { code: "fur", name: "Friulian", source: true, target: true },
    Language { code: "ff", name: "Fulani", source: true, target: true },
    Language { code: "gaa", name: "Ga", source: true, target: true },
    Language { code: "gl", name: "Galician", source: true, target: true },
    Language { code: "ka", name: "Georgian", source: true, target: true },
    Language { code: "de", name: "German", source: true, target: true },
    Language { code: "el", name: "Greek", source: true, target: true },
    Language { code: "gn", name: "Guarani", source: true, target: true },
    Language { code: "gu", name: "Gujarati", source: true, target: true },
    Language { code: "ht", name: "Haitian Creole", source: true, target: true },
    Language { code: "cnh", name: "Hakha Chin", source: true, target: true },
    Language { code: "ha", name: "Hausa", source: true, target: true },
    Language { code: "haw", name: "Hawaiian", source: true, target: true },
    Language { code: "he", name: "Hebrew", source: true, target: true },
    Language { code: "hil", name: "Hiligaynon", source: true, target: true },
    Language { code: "hi", name: "Hindi", source: true, target: true },
    Language { code: "hmn", name: "Hmong", source: true, target: true },
    Language { code: "hu", name: "Hungarian", source: true, target: true },
    Language { code: "hrx", name: "Hunsrik", source: true, target: true },
    Language { code: "iba", name: "Iban", source: true, target: true },
    Language { code: "is", name: "Icelandic", source: true, target: true },
    Language { code: "ig", name: "Igbo", source: true, target: true },
    Language { code: "ilo", name: "Ilocano", source: true, target: true },
    Language { code: "id", name: "Indonesian", source: true, target: true },
    Language { code: "iu-Latn", name: "Inuktut (Latin)", source: true, target: true },
    Language { code: "iu", name: "Inuktut (Syllabics)", source: true, target: true },
    Language { code: "ga", name: "Irish", source: true, target: true },
    Language { code: "it", name: "Italian", source: true, target: true },
    Language { code: "jam", name: "Jamaican Patois", source: true, target: true },
    Language { code: "ja", name: "Japanese", source: true, target: true },
    Language { code: "jv", name: "Javanese", source: true, target: true },
    Language { code: "kac", name: "Jingpo", source: true, target: true },
    Language { code: "kl", name: "Kalaallisut", source: true, target: true },
    Language { code: "kn", name: "Kannada", source: true, target: true },
    Language { code: "kr", name: "Kanuri", source: true, target: true },
    Language { code: "pam", name: "Kapampangan", source: true, target: true },
    Language { code: "kk", name: "Kazakh", source: true, target: true },
    Language { code: "kha", name: "Khasi", source: true, target: true },
    Language { code: "km", name: "Khmer", source: true, target: true },
    Language { code: "cgg", name: "Kiga", source: true, target: true },
    Language { code: "kg", name: "Kikongo", source: true, target: true },
    Language { code: "rw", name: "Kinyarwanda", source: true, target: true },
    Language { code: "ktu", name: "Kituba", source: true, target: true },
    Language { code: "trp", name: "Kokborok", source: true, target: true },
    Language { code: "kv", name: "Komi", source: true, target: true },
    Language { code: "gom", name: "Konkani", source: true, target: true },
    Language { code: "ko", name: "Korean", source: true, target: true },
    Language { code: "kri", name: "Krio", source: true, target: true },
    Language { code: "ku", name: "Kurdish (Kurmanji)", source: true, target: true },
    Language { code: "ckb", name: "Kurdish (Sorani)", source: true, target: true },
    Language { code: "ky", name: "Kyrgyz", source: true, target: true },
    Language { code: "lo", name: "Lao", source: true, target: true },
    Language { code: "ltg", name: "Latgalian", source: true, target: true },
    Language { code: "la", name: "Latin", source: true, target: true },
    Language { code: "lv", name: "Latvian", source: true, target: true },
    Language { code: "lij", name: "Ligurian", source: true, target: true },
    Language { code: "li", name: "Limburgish", source: true, target: true },
    Language { code: "ln", name: "Lingala", source: true, target: true },
    Language { code: "lt", name: "Lithuanian", source: true, target: true },
    Language { code: "lmo", name: "Lombard", source: true, target: true },
    Language { code: "lg", name: "Luganda", source: true, target: true },
    Language { code: "luo", name: "Luo", source: true, target: true },
    Language { code: "lb", name: "Luxembourgish", source: true, target: true },
    Language { code: "mk", name: "Macedonian", source: true, target: true },
    Language { code: "mad", name: "Madurese", source: true, target: true },
    Language { code: "mai", name: "Maithili", source: true, target: true },
    Language { code: "mak", name: "Makassar", source: true, target: true },
    Language { code: "mg", name: "Malagasy", source: true, target: true },
    Language { code: "ms", name: "Malay", source: true, target: true },
    Language { code: "ms-Arab", name: "Malay (Jawi)", source: true, target: true },
    Language { code: "ml", name: "Malayalam", source: true, target: true },
    Language { code: "mt", name: "Maltese", source: true, target: true },
    Language { code: "mam", name: "Mam", source: true, target: true },
    Language { code: "gv", name: "Manx", source: true, target: true },
    Language { code: "mi", name: "Maori", source: true, target: true },
    Language { code: "mr", name: "Marathi", source: true, target: true },
    Language { code: "mh", name: "Marshallese", source: true, target: true },
    Language { code: "mwr", name: "Marwadi", source: true, target: true },
    Language { code: "mfe", name: "Mauritian Creole", source: true, target: true },
    Language { code: "chm", name: "Meadow Mari", source: true, target: true },
    Language { code: "mni-Mtei", name: "Meiteilon (Manipuri)", source: true, target: true },
    Language { code: "min", name: "Minang", source: true, target: true },
    Language { code: "lus", name: "Mizo", source: true, target: true },
    Language { code: "mn", name: "Mongolian", source: true, target: true },
    Language { code: "my", name: "Myanmar (Burmese)", source: true, target: true },
    Language { code: "nhe", name: "Nahuatl (Eastern Huasteca)", source: true, target: true },
    Language { code: "ndc-ZW", name: "Ndau", source: true, target: true },
    Language { code: "nr", name: "Ndebele (South)", source: true, target: true },
    Language { code: "new", name: "Nepalbhasa (Newari)", source: true, target: true },
    Language { code: "ne", name: "Nepali", source: true, target: true },
    Language { code: "bm-Nkoo", name: "NKo", source: true, target: true },
    Language { code: "nb", name: "Norwegian", source: true, target: true },
    Language { code: "nus", name: "Nuer", source: true, target: true },
    Language { code: "oc", name: "Occitan", source: true, target: true },
    Language { code: "or", name: "Odia (Oriya)", source: true, target: true },
    Language { code: "om", name: "Oromo", source: true, target: true },
    Language { code: "os", name: "Ossetian", source: true, target: true },
    Language { code: "pag", name: "Pangasinan", source: true, target: true },
    Language { code: "pap", name: "Papiamento", source: true, target: true },
    Language { code: "ps", name: "Pashto", source: true, target: true },
    Language { code: "fa", name: "Persian", source: true, target: true },
    Language { code: "pl", name: "Polish", source: true, target: true },
    Language { code: "pt", name: "Portuguese (Brazil)", source: true, target: true },
    Language { code: "pt-PT", name: "Portuguese (Portugal)", source: true, target: true },
    Language { code: "pa", name: "Punjabi (Gurmukhi)", source: true, target: true },
    Language { code: "pa-Arab", name: "Punjabi (Shahmukhi)", source: true, target: true },
    Language { code: "qu", name: "Quechua", source: true, target: true },
    Language { code: "kek", name: "Qʼeqchiʼ", source: true, target: true },
    Language { code: "rom", name: "Romani", source: true, target: true },
    Language { code: "ro", name: "Romanian", source: true, target: true },
    Language { code: "rn", name: "Rundi", source: true, target: true },
    Language { code: "ru", name: "Russian", source: true, target: true },
    Language { code: "se", name: "Sami (North)", source: true, target: true },
    Language { code: "sm", name: "Samoan", source: true, target: true },
    Language { code: "sg", name: "Sango", source: true, target: true },
    Language { code: "sa", name: "Sanskrit", source: true, target: true },
    Language { code: "sat-Latn", name: "Santali (Latin)", source: true, target: true },
    Language { code: "sat", name: "Santali (Ol Chiki)", source: true, target: true },
    Language { code: "gd", name: "Scots Gaelic", source: true, target: true },
    Language { code: "nso", name: "Sepedi", source: true, target: true },
    Language { code: "sr", name: "Serbian", source: true, target: true },
    Language { code: "st", name: "Sesotho", source: true, target: true },
    Language { code: "crs", name: "Seychellois Creole", source: true, target: true },
    Language { code: "shn", name: "Shan", source: true, target: true },
    Language { code: "sn", name: "Shona", source: true, target: true },
    Language { code: "scn", name: "Sicilian", source: true, target: true },
    Language { code: "szl", name: "Silesian", source: true, target: true },
    Language { code: "sd", name: "Sindhi", source: true, target: true },
    Language { code: "si", name: "Sinhala", source: true, target: true },
    Language { code: "sk", name: "Slovak", source: true, target: true },
    Language { code: "sl", name: "Slovenian", source: true, target: true },
    Language { code: "so", name: "Somali", source: true, target: true },
    Language { code: "es", name: "Spanish", source: true, target: true },
    Language { code: "su", name: "Sundanese", source: true, target: true },
    Language { code: "sus", name: "Susu", source: true, target: true },
    Language { code: "sw", name: "Swahili", source: true, target: true },
    Language { code: "ss", name: "Swati", source: true, target: true },
    Language { code: "sv", name: "Swedish", source: true, target: true },
    Language { code: "ty", name: "Tahitian", source: true, target: true },
    Language { code: "tg", name: "Tajik", source: true, target: true },
    Language { code: "ber-Latn", name: "Tamazight", source: true, target: true },
    Language { code: "ber", name: "Tamazight (Tifinagh)", source: true, target: true },
    Language { code: "ta", name: "Tamil", source: true, target: true },
    Language { code: "tt", name: "Tatar", source: true, target: true },
    Language { code: "te", name: "Telugu", source: true, target: true },
    Language { code: "tet", name: "Tetum", source: true, target: true },
    Language { code: "th", name: "Thai", source: true, target: true },
    Language { code: "bo", name: "Tibetan", source: true, target: true },
    Language { code: "ti", name: "Tigrinya", source: true, target: true },
    Language { code: "tiv", name: "Tiv", source: true, target: true },
    Language { code: "tpi", name: "Tok Pisin", source: true, target: true },
    Language { code: "to", name: "Tongan", source: true, target: true },
    Language { code: "lua", name: "Tshiluba", source: true, target: true },
    Language { code: "ts", name: "Tsonga", source: true, target: true },
    Language { code: "tn", name: "Tswana", source: true, target: true },
    Language { code: "tcy", name: "Tulu", source: true, target: true },
    Language { code: "tum", name: "Tumbuka", source: true, target: true },
    Language { code: "tr", name: "Turkish", source: true, target: true },
    Language { code: "tk", name: "Turkmen", source: true, target: true },
    Language { code: "tyv", name: "Tuvan", source: true, target: true },
    Language { code: "ak", name: "Twi", source: true, target: true },
    Language { code: "udm", name: "Udmurt", source: true, target: true },
    Language { code: "uk", name: "Ukrainian", source: true, target: true },
    Language { code: "ur", name: "Urdu", source: true, target: true },
    Language { code: "ug", name: "Uyghur", source: true, target: true },
    Language { code: "uz", name: "Uzbek", source: true, target: true },
    Language { code: "ve", name: "Venda", source: true, target: true },
    Language { code: "vec", name: "Venetian", source: true, target: true },
    Language { code: "vi", name: "Vietnamese", source: true, target: true },
    Language { code: "war", name: "Waray", source: true, target: true },
    Language { code: "cy", name: "Welsh", source: true, target: true },
    Language { code: "wo", name: "Wolof", source: true, target: true },
    Language { code: "xh", name: "Xhosa", source: true, target: true },
    Language { code: "sah", name: "Yakut", source: true, target: true },
    Language { code: "yi", name: "Yiddish", source: true, target: true },
    Language { code: "yo", name: "Yoruba", source: true, target: true },
    Language { code: "yua", name: "Yucatec Maya", source: true, target: true },
    Language { code: "zap", name: "Zapotec", source: true, target: true },
    Language { code: "zu", name: "Zulu", source: true, target: true },
];

pub fn languages() -> &'static [Language] {
    LANGUAGES
}

pub(super) fn language_code(code: &str) -> &str {
    match code {
        "he" => "iw",
        "fil" => "tl",
        "nb" => "no",
        "jv" => "jw",
        _ => code,
    }
}

pub(super) fn normalize_language(code: &str) -> String {
    let code = code.trim();
    match code.to_ascii_lowercase().as_str() {
        "iw" | "he" => "he".into(),
        "tl" | "fil" => "fil".into(),
        "no" | "nb" => "nb".into(),
        "jw" | "jv" => "jv".into(),
        "zh" | "zh-cn" | "zh-hans" => "zh-CN".into(),
        "zh-tw" | "zh-hant" => "zh-TW".into(),
        _ => LANGUAGES
            .iter()
            .find(|language| language.code.eq_ignore_ascii_case(code))
            .map_or_else(|| code.to_string(), |language| language.code.to_string()),
    }
}

pub(super) fn request(
    client: &Client,
    texts: &[&str],
    source: &str,
    target: &str,
) -> RequestBuilder {
    client
        .post("https://translate-pa.googleapis.com/v1/translateHtml")
        .header("Content-Type", "application/json+protobuf")
        .json(&json!([
            [texts, language_code(source), language_code(target)],
            "te_lib"
        ]))
        // Public Translate Element frontend key used by the existing client.
        .header("X-Goog-Api-Key", "AIzaSyATBXajvzQLTDHEQbcpq0Ihe0vWDHmO520")
}

pub(super) fn parse(data: &[u8], count: usize, source: &str) -> Result<Vec<SegmentResult>, String> {
    let malformed = || "The translation service returned an invalid response.".to_string();
    let root: Value = serde_json::from_slice(data).map_err(|_| malformed())?;
    let root = root.as_array().ok_or_else(malformed)?;
    let texts = root
        .first()
        .and_then(Value::as_array)
        .filter(|texts| texts.len() == count)
        .ok_or_else(malformed)?;
    let detected = if source == "auto" {
        Some(
            root.get(1)
                .and_then(Value::as_array)
                .filter(|codes| codes.len() == count)
                .ok_or_else(malformed)?,
        )
    } else {
        None
    };
    texts
        .iter()
        .enumerate()
        .map(|(index, text)| {
            let text = text.as_str().ok_or_else(malformed)?.to_string();
            let detection = if let Some(codes) = detected {
                let code = codes[index]
                    .as_str()
                    .filter(|code| !code.trim().is_empty())
                    .ok_or_else(malformed)?;
                Some(LanguageDetection {
                    raw_language: code.to_string(),
                    language: normalize_language(code),
                    score: None,
                })
            } else {
                None
            };
            Ok(SegmentResult { text, detection })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_and_target_catalogs_are_separate_and_keep_legacy_chinese_source() {
        let languages = languages();
        assert_eq!(
            languages.iter().filter(|language| language.target).count(),
            249
        );
        assert_eq!(
            languages.iter().filter(|language| language.source).count(),
            249
        );
        for code in [
            "zh-CN", "zh-TW", "he", "fil", "nb", "jv", "pt", "pt-PT", "eo", "sw",
        ] {
            assert!(
                languages
                    .iter()
                    .any(|language| language.code == code && language.source && language.target),
                "{code}"
            );
        }
        let mut codes: Vec<_> = languages.iter().map(|language| language.code).collect();
        codes.sort_unstable();
        codes.dedup();
        assert_eq!(codes.len(), languages.len());
        assert!(languages
            .iter()
            .all(|language| !language.name.is_empty() && language.code != "auto"));
    }

    #[test]
    fn aliases_are_normalized_without_dropping_unknown_languages() {
        for (provider, canonical) in [
            ("iw", "he"),
            ("tl", "fil"),
            ("no", "nb"),
            ("jw", "jv"),
            ("zh-Hant", "zh-TW"),
            ("zh-Hans", "zh-CN"),
            ("x-custom", "x-custom"),
        ] {
            assert_eq!(normalize_language(provider), canonical);
        }
        for (canonical, provider) in [
            ("he", "iw"),
            ("fil", "tl"),
            ("nb", "no"),
            ("jv", "jw"),
            ("zh-TW", "zh-TW"),
            ("pt-PT", "pt-PT"),
        ] {
            assert_eq!(language_code(canonical), provider);
        }
        assert_eq!(normalize_language("ku"), "ku");
        assert_eq!(language_code("ckb"), "ckb");
    }

    #[test]
    fn request_sends_prepared_html_without_escaping_it_again() {
        let client = Client::new();
        let request = request(
            &client,
            &["<code translate=\"no\">npm run build</code> &amp; text"],
            "he",
            "zh-TW",
        )
        .build()
        .unwrap();
        assert_eq!(request.method(), reqwest::Method::POST);
        assert_eq!(
            request.url().as_str(),
            "https://translate-pa.googleapis.com/v1/translateHtml"
        );
        assert_eq!(
            request.headers()["Content-Type"],
            "application/json+protobuf"
        );
        assert!(request.headers().contains_key("X-Goog-Api-Key"));
        let body: Value =
            serde_json::from_slice(request.body().unwrap().as_bytes().unwrap()).unwrap();
        assert_eq!(
            body,
            json!([
                [
                    ["<code translate=\"no\">npm run build</code> &amp; text"],
                    "iw",
                    "zh-TW"
                ],
                "te_lib"
            ])
        );
    }

    #[test]
    fn parses_auto_detection_without_changing_prepared_html_or_entities() {
        let rows = parse(
            br#"[["<span translate=\"no\">build</span> &amp; text","Other"],["iw","x-custom"]]"#,
            2,
            "auto",
        )
        .unwrap();
        assert_eq!(
            rows[0].text,
            "<span translate=\"no\">build</span> &amp; text"
        );
        let first = rows[0].detection.as_ref().unwrap();
        assert_eq!(first.raw_language, "iw");
        assert_eq!(first.language, "he");
        assert_eq!(first.score, None);
        let second = rows[1].detection.as_ref().unwrap();
        assert_eq!(second.raw_language, "x-custom");
        assert_eq!(second.language, "x-custom");
    }

    #[test]
    fn explicit_source_does_not_require_detection_metadata() {
        let rows = parse(br#"[["Hello"]]"#, 1, "zh-TW").unwrap();
        assert_eq!(rows[0].text, "Hello");
        assert!(rows[0].detection.is_none());
    }

    #[test]
    fn rejects_malformed_or_misaligned_results() {
        for (data, count) in [
            (b"not json".as_slice(), 1),
            (br#"{}"#.as_slice(), 1),
            (br#"[["one"],["en"]]"#.as_slice(), 2),
            (br#"[[42],["en"]]"#.as_slice(), 1),
            (br#"[["one"]]"#.as_slice(), 1),
            (br#"[["one"],[]]"#.as_slice(), 1),
            (br#"[["one"],[42]]"#.as_slice(), 1),
            (br#"[["one"],[""]]"#.as_slice(), 1),
        ] {
            assert!(parse(data, count, "auto").is_err(), "{data:?}");
        }
    }
}
