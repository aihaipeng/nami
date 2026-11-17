use std::collections::HashMap;
use std::ops::Range;

pub(super) struct Unit<'a> {
    pub original: &'a str,
    pub prepared: String,
    pub translate: bool,
    protections: Vec<&'a str>,
}

pub(super) fn units(text: &str, auto: bool) -> Vec<Unit<'_>> {
    let mut result = Vec::new();
    let mut start = 0;
    let mut fence: Option<(u8, usize)> = None;
    let mut sentence_count = 0;
    while start < text.len() {
        let previous_count = result.len();
        let mut end = start;
        while end < text.len() && !matches!(text.as_bytes()[end], b'\r' | b'\n') {
            end += 1;
        }
        let mut next = end;
        if next < text.len() {
            next += 1;
            if text.as_bytes()[end] == b'\r' && text.as_bytes().get(next) == Some(&b'\n') {
                next += 1;
            }
        }
        let line = &text[start..end];
        let marker = fence_marker(line);
        if let Some((character, width)) = fence {
            literal(&mut result, &text[start..next]);
            if marker.is_some_and(|(found, count, rest)| {
                found == character && count >= width && rest.trim().is_empty()
            }) {
                fence = None;
            }
        } else if let Some((character, width, _)) = marker {
            fence = Some((character, width));
            literal(&mut result, &text[start..next]);
        } else {
            split_line(&mut result, line, auto);
            literal(&mut result, &text[end..next]);
        }
        sentence_count += result[previous_count..]
            .iter()
            .filter(|unit| unit.translate)
            .count();
        // Dense short sentences retain line context instead of multiplying network batches.
        if auto && sentence_count > 2_048 {
            return units(text, false);
        }
        start = next;
    }
    result
}

fn fence_marker(line: &str) -> Option<(u8, usize, &str)> {
    let line = line.trim_start();
    let character = *line.as_bytes().first()?;
    if !matches!(character, b'`' | b'~') {
        return None;
    }
    let width = line.bytes().take_while(|byte| *byte == character).count();
    (width >= 3).then(|| (character, width, &line[width..]))
}

fn literal<'a>(result: &mut Vec<Unit<'a>>, original: &'a str) {
    if !original.is_empty() {
        result.push(Unit {
            original,
            prepared: String::new(),
            translate: false,
            protections: Vec::new(),
        });
    }
}

fn split_line<'a>(result: &mut Vec<Unit<'a>>, line: &'a str, auto: bool) {
    let ranges = protected_ranges(line);
    let mut start = 0;
    let mut index = 0;
    let mut protection = 0;
    while index < line.len() {
        if let Some(range) = ranges.get(protection) {
            if index == range.start {
                index = range.end;
                protection += 1;
                continue;
            }
        }
        let character = line[index..].chars().next().unwrap();
        index += character.len_utf8();
        let sentence_end = matches!(character, '。' | '！' | '？' | '!' | '?')
            || character == '.'
                && (index == line.len() || line[index..].starts_with(char::is_whitespace))
                && !abbreviation(&line[..index]);
        if auto && sentence_end {
            while index < line.len() {
                let closing = line[index..].chars().next().unwrap();
                if !matches!(
                    closing,
                    '.' | '!' | '?' | '。' | '！' | '？' | '\'' | '"' | '’' | '”' | ')' | ']' | '}'
                ) {
                    break;
                }
                index += closing.len_utf8();
            }
            push_unit(result, &line[start..index]);
            start = index;
        }
    }
    push_unit(result, &line[start..]);
}

fn abbreviation(prefix: &str) -> bool {
    let bytes = prefix.as_bytes();
    let mut start = bytes.len();
    while start > 0
        && bytes.len() - start < 8
        && (bytes[start - 1].is_ascii_alphabetic() || bytes[start - 1] == b'.')
    {
        start -= 1;
    }
    let word = prefix[start..].to_ascii_lowercase();
    matches!(
        word.as_str(),
        "mr." | "mrs." | "ms." | "dr." | "prof." | "sr." | "jr." | "e.g." | "i.e." | "vs." | "etc."
    ) || word.len() == 2 && word.as_bytes()[0].is_ascii_alphabetic()
}

fn push_unit<'a>(result: &mut Vec<Unit<'a>>, text: &'a str) {
    let body = text.trim();
    if body.is_empty() {
        literal(result, text);
        return;
    }
    let leading = text.len() - text.trim_start().len();
    literal(result, &text[..leading]);
    let ranges = protected_ranges(body);
    let mut prepared = String::new();
    let mut protections = Vec::with_capacity(ranges.len());
    let mut cursor = 0;
    let mut has_letters = false;
    for range in ranges {
        let ordinary = &body[cursor..range.start];
        has_letters |= ordinary.chars().any(char::is_alphabetic);
        escape_into(&mut prepared, ordinary);
        prepared.push_str(&format!(
            "<span translate=\"no\" id=\"nami-code-{}\">",
            protections.len()
        ));
        let original = &body[range.clone()];
        escape_into(&mut prepared, original);
        prepared.push_str("</span>");
        protections.push(original);
        cursor = range.end;
    }
    let ordinary = &body[cursor..];
    has_letters |= ordinary.chars().any(char::is_alphabetic);
    escape_into(&mut prepared, ordinary);
    result.push(Unit {
        original: body,
        prepared,
        translate: has_letters,
        protections,
    });
    literal(result, &text[leading + body.len()..]);
}

fn escape_into(output: &mut String, text: &str) {
    for character in text.chars() {
        match character {
            '&' => output.push_str("&amp;"),
            '<' => output.push_str("&lt;"),
            '>' => output.push_str("&gt;"),
            _ => output.push(character),
        }
    }
}

fn protected_ranges(text: &str) -> Vec<Range<usize>> {
    let bytes = text.as_bytes();
    let mut runs = Vec::new();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'`' {
            let start = index;
            while bytes.get(index) == Some(&b'`') {
                index += 1;
            }
            runs.push((start, index));
        } else {
            index += 1;
        }
    }
    let mut next_by_width = HashMap::new();
    let mut ends = HashMap::new();
    for &(start, end) in runs.iter().rev() {
        if let Some(&(_, closing_end)) = next_by_width.get(&(end - start)) {
            ends.insert(start, closing_end);
        }
        next_by_width.insert(end - start, (start, end));
    }
    let mut ranges = Vec::new();
    index = 0;
    while index < bytes.len() {
        let end = ends
            .get(&index)
            .copied()
            .or_else(|| url_end(text, index))
            .or_else(|| command_end(text, index));
        if let Some(end) = end {
            ranges.push(index..end);
            index = end;
        } else {
            index += text[index..].chars().next().unwrap().len_utf8();
        }
    }
    ranges
}

fn token_boundary(text: &str, index: usize) -> bool {
    index == 0
        || text[..index]
            .chars()
            .next_back()
            .is_some_and(|character| !character.is_ascii_alphanumeric() && character != '_')
}

fn url_end(text: &str, index: usize) -> Option<usize> {
    let rest = &text[index..];
    if !token_boundary(text, index)
        || !["https://", "http://", "www."]
            .iter()
            .any(|prefix| rest.starts_with(prefix))
    {
        return None;
    }
    let mut end = index;
    for character in rest.chars() {
        if character.is_whitespace()
            || matches!(
                character,
                '<' | '>' | '"' | '\'' | '`' | '。' | '！' | '？' | '，' | '；'
            )
        {
            break;
        }
        end += character.len_utf8();
    }
    let mut balance = [0_i32; 3];
    for byte in text[index..end].bytes() {
        match byte {
            b'(' => balance[0] += 1,
            b')' => balance[0] -= 1,
            b'[' => balance[1] += 1,
            b']' => balance[1] -= 1,
            b'{' => balance[2] += 1,
            b'}' => balance[2] -= 1,
            _ => {}
        }
    }
    while end > index {
        match text.as_bytes()[end - 1] {
            b'.' | b',' | b'!' | b'?' | b';' | b':' => end -= 1,
            byte @ (b')' | b']' | b'}') => {
                let kind = match byte {
                    b')' => 0,
                    b']' => 1,
                    _ => 2,
                };
                if balance[kind] >= 0 {
                    break;
                }
                balance[kind] += 1;
                end -= 1;
            }
            _ => break,
        }
    }
    (end > index).then_some(end)
}

fn command_end(text: &str, index: usize) -> Option<usize> {
    if !token_boundary(text, index) {
        return None;
    }
    let rest = &text[index..];
    let executable = ["npm", "pnpm", "yarn"]
        .iter()
        .find(|name| rest.starts_with(**name))?;
    let mut end = index + executable.len();
    let whitespace = text[end..].len() - text[end..].trim_start_matches([' ', '\t']).len();
    if whitespace == 0 {
        return None;
    }
    end += whitespace;
    if !text[end..].starts_with("run") {
        return None;
    }
    end += 3;
    let whitespace = text[end..].len() - text[end..].trim_start_matches([' ', '\t']).len();
    if whitespace == 0 {
        return None;
    }
    end += whitespace;
    let script_start = end;
    while text.as_bytes().get(end).is_some_and(|byte| {
        byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b':' | b'.')
    }) {
        end += 1;
    }
    while end > script_start && text.as_bytes()[end - 1] == b'.' {
        end -= 1;
    }
    (end > script_start).then_some(end)
}

impl Unit<'_> {
    pub(super) fn restore(&self, translated: &str) -> Result<String, String> {
        if !self.translate {
            return Ok(self.original.to_string());
        }
        let malformed = || "The translation service returned invalid protected text.".to_string();
        let mut restored = String::new();
        let mut cursor = 0;
        let mut seen = vec![false; self.protections.len()];
        while let Some(relative) = translated[cursor..].find('<') {
            let opening = cursor + relative;
            restored.push_str(&html_escape::decode_html_entities(
                &translated[cursor..opening],
            ));
            let tag_end = translated[opening..]
                .find('>')
                .map(|relative| opening + relative)
                .ok_or_else(malformed)?;
            let id = protection_id(&translated[opening + 1..tag_end]).ok_or_else(malformed)?;
            let index = id
                .strip_prefix("nami-code-")
                .and_then(|index| index.parse::<usize>().ok())
                .ok_or_else(malformed)?;
            if id != format!("nami-code-{index}") || seen.get(index).is_none_or(|seen| *seen) {
                return Err(malformed());
            }
            let content_start = tag_end + 1;
            let closing = translated[content_start..]
                .find('<')
                .map(|relative| content_start + relative)
                .ok_or_else(malformed)?;
            let close_end = translated[closing..]
                .find('>')
                .map(|relative| closing + relative)
                .ok_or_else(malformed)?;
            if !translated[closing + 1..close_end]
                .trim()
                .eq_ignore_ascii_case("/span")
            {
                return Err(malformed());
            }
            restored.push_str(self.protections[index]);
            seen[index] = true;
            cursor = close_end + 1;
        }
        if seen.iter().any(|seen| !seen) {
            return Err(malformed());
        }
        restored.push_str(&html_escape::decode_html_entities(&translated[cursor..]));
        Ok(restored)
    }
}

fn protection_id(tag: &str) -> Option<&str> {
    let tag = tag.trim();
    if !tag.get(..4)?.eq_ignore_ascii_case("span") {
        return None;
    }
    let mut attributes = &tag[4..];
    if !attributes.starts_with(char::is_whitespace) {
        return None;
    }
    let mut id = None;
    let mut translate_seen = false;
    loop {
        attributes = attributes.trim_start();
        if attributes.is_empty() {
            return id;
        }
        let name_end = attributes.find(|character: char| !character.is_ascii_alphabetic())?;
        let name = &attributes[..name_end];
        attributes = attributes[name_end..]
            .trim_start()
            .strip_prefix('=')?
            .trim_start();
        let quote = attributes.chars().next()?;
        if !matches!(quote, '\'' | '"') {
            return None;
        }
        attributes = &attributes[1..];
        let value_end = attributes.find(quote)?;
        let value = &attributes[..value_end];
        attributes = &attributes[value_end + 1..];
        if !attributes.is_empty() && !attributes.starts_with(char::is_whitespace) {
            return None;
        }
        if name.eq_ignore_ascii_case("id") && id.is_none() {
            id = Some(value);
        } else if name.eq_ignore_ascii_case("translate")
            && !translate_seen
            && value.eq_ignore_ascii_case("no")
        {
            translate_seen = true;
        } else {
            return None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn translated(text: &str, auto: bool) -> Vec<String> {
        units(text, auto)
            .into_iter()
            .filter(|unit| unit.translate)
            .map(|unit| unit.original.to_string())
            .collect()
    }

    #[test]
    fn separates_natural_sentences_without_splitting_scripts() {
        assert_eq!(
            translated(
                "今天的会议取消了。Please bring the report. 明日の会議は中止です。",
                true
            ),
            [
                "今天的会议取消了。",
                "Please bring the report.",
                "明日の会議は中止です。"
            ]
        );
        assert_eq!(
            translated("请检查dependency array，然后执行命令。", true),
            ["请检查dependency array，然后执行命令。"]
        );
    }

    #[test]
    fn manual_source_keeps_paragraph_context() {
        assert_eq!(
            translated("中文。English.\nNext line!", false),
            ["中文。English.", "Next line!"]
        );
    }

    #[test]
    fn preserves_crlf_blank_lines_and_whitespace_exactly() {
        let original = "  中文。 \t\r\n\r\n  English!  \n";
        let units = units(original, true);
        assert_eq!(
            units.iter().map(|unit| unit.original).collect::<String>(),
            original
        );
        assert_eq!(translated(original, true), ["中文。", "English!"]);
        assert!(units
            .iter()
            .filter(|unit| !unit.translate)
            .all(|unit| unit.restore("ignored").unwrap() == unit.original));
    }

    #[test]
    fn abbreviations_and_decimals_do_not_create_false_sentences() {
        assert_eq!(
            translated(
                "Dr. Smith met Mr. Jones, e.g. at 3.14 pm. Next sentence.",
                true
            ),
            [
                "Dr. Smith met Mr. Jones, e.g. at 3.14 pm.",
                "Next sentence."
            ]
        );
    }

    #[test]
    fn protects_urls_without_splitting_their_dots_or_query() {
        let units = units("Visit https://example.com/a?q=x.y. Then return.", true);
        let first = units.iter().find(|unit| unit.translate).unwrap();
        assert_eq!(first.original, "Visit https://example.com/a?q=x.y.");
        assert!(first
            .prepared
            .contains("id=\"nami-code-0\">https://example.com/a?q=x.y</span>."));
    }

    #[test]
    fn fenced_code_is_literal_including_unclosed_fences() {
        for input in [
            "```rust\nlet x = 1.0;\n```\nTranslate this.",
            "~~~\ncode!\n~~~\nTranslate this.",
        ] {
            assert_eq!(translated(input, true), ["Translate this."]);
            assert_eq!(
                units(input, true)
                    .iter()
                    .map(|unit| unit.original)
                    .collect::<String>(),
                input
            );
        }
        assert_eq!(
            translated("Before.\n```\nNever translate this.", true),
            ["Before."]
        );
    }

    #[test]
    fn inline_code_retains_backticks_and_modified_response_content() {
        let units = units("请执行 `npm run build`。", true);
        let unit = units.iter().find(|unit| unit.translate).unwrap();
        assert_eq!(
            unit.restore("Run <span translate=\"no\" id=\"nami-code-0\">\"npm run\"</span>.")
                .unwrap(),
            "Run `npm run build`."
        );
    }

    #[test]
    fn multiple_backticks_protect_embedded_single_backticks() {
        let units = units("Use ``a ` b`` safely.", true);
        let unit = units.iter().find(|unit| unit.translate).unwrap();
        assert_eq!(unit.restore(&unit.prepared).unwrap(), unit.original);
        assert!(unit.prepared.contains(">``a ` b``</span>"));
    }

    #[test]
    fn npm_pnpm_and_yarn_commands_keep_the_whole_script_name() {
        for command in ["npm run build", "pnpm run test:ui", "yarn run lint-check"] {
            let text = format!("请执行 {command}。");
            let units = units(&text, true);
            let unit = units.iter().find(|unit| unit.translate).unwrap();
            assert!(unit.prepared.contains(&format!(">{command}</span>")));
            assert_eq!(unit.restore(&unit.prepared).unwrap(), text);
        }
    }

    #[test]
    fn literal_code_urls_numbers_and_symbols_do_not_need_network() {
        for text in [
            "12345 2026-11-17",
            "`npm run build`",
            "npm run build",
            "https://example.com/path",
            "😀✨",
        ] {
            assert!(
                units(text, true).iter().all(|unit| !unit.translate),
                "{text}"
            );
        }
    }

    #[test]
    fn escapes_user_html_and_decodes_once() {
        let units = units("Use <script>alert(1)</script> &lt;tag&gt;.", true);
        let unit = units.iter().find(|unit| unit.translate).unwrap();
        assert!(!unit.prepared.contains("<script>"));
        assert_eq!(unit.restore(&unit.prepared).unwrap(), unit.original);
    }

    #[test]
    fn escaped_fake_protection_ids_are_plain_text() {
        let input = "Keep <span id=\"nami-code-0\">fake</span> and `real`.";
        let units = units(input, true);
        let unit = units.iter().find(|unit| unit.translate).unwrap();
        assert_eq!(unit.restore(&unit.prepared).unwrap(), input);
    }

    #[test]
    fn rejects_missing_duplicate_or_unknown_protections() {
        let units = units("Use `first` and `second`.", true);
        let unit = units.iter().find(|unit| unit.translate).unwrap();
        for response in [
            "Use no code.",
            "<span id='nami-code-00'>a</span><span id='nami-code-1'>b</span>",
            "<span id='nami-code-18446744073709551616000'>a</span>",
            "<span id=\"nami-code-0\">a</span><span id=\"nami-code-0\">b</span>",
            "<span id=\"nami-code-0\">a</span><span id=\"nami-code-9\">b</span>",
        ] {
            assert!(unit.restore(response).is_err(), "{response}");
        }
    }

    #[test]
    fn restores_reordered_protections_in_the_translated_order() {
        let units = units("Use `first` and `second`.", true);
        let unit = units.iter().find(|unit| unit.translate).unwrap();
        let response = "Before <span id='nami-code-1'>modified second</span>, put <span id='nami-code-0'>modified first</span>.";
        assert_eq!(
            unit.restore(response).unwrap(),
            "Before `second`, put `first`."
        );
    }

    #[test]
    fn rejects_unclosed_nested_or_unapproved_html() {
        let units = units("Use `code`.", true);
        let unit = units.iter().find(|unit| unit.translate).unwrap();
        for response in [
            "<span id=\"nami-code-0\">code",
            "<script>x</script>",
            "<b>text</b>",
            "<span id=\"nami-code-0\" onclick=\"x\">code</span>",
            "<span id=\"nami-code-0\"><span>code</span></span>",
        ] {
            assert!(unit.restore(response).is_err(), "{response}");
        }
    }

    #[test]
    fn supports_safe_attribute_order_and_single_quotes() {
        let units = units("Use `code`.", true);
        let unit = units.iter().find(|unit| unit.translate).unwrap();
        assert_eq!(
            unit.restore("Use <span id='nami-code-0' translate='no'>modified</span>.")
                .unwrap(),
            "Use `code`."
        );
    }

    #[test]
    fn protected_entities_are_never_decoded_twice() {
        let units = units("Use `&lt;tag&gt;` &amp; text.", true);
        let unit = units.iter().find(|unit| unit.translate).unwrap();
        assert_eq!(unit.restore(&unit.prepared).unwrap(), unit.original);
    }

    #[test]
    fn empty_and_unicode_inputs_never_lose_bytes() {
        for input in [
            "",
            "\r",
            "中文😀。\r\nمرحبا!",
            "é e\u{301}\tहिन्दी।",
            "`unclosed 中文。",
        ] {
            assert_eq!(
                units(input, true)
                    .iter()
                    .map(|unit| unit.original)
                    .collect::<String>(),
                input
            );
        }
    }

    #[test]
    fn fenced_content_does_not_close_on_shorter_or_inline_markers() {
        let text = "````\n```\nnot closed.\n````\nAfter.";
        assert_eq!(translated(text, true), ["After."]);
        assert_eq!(
            translated("Before ``` inline. After.", true),
            ["Before ``` inline.", "After."]
        );
    }

    #[test]
    fn valid_parentheses_inside_urls_remain_protected() {
        let units = units("Visit https://example.com/wiki/Test_(example). Done.", true);
        let unit = units.iter().find(|unit| unit.translate).unwrap();
        assert!(unit
            .prepared
            .contains(">https://example.com/wiki/Test_(example)</span>."));
        assert_eq!(unit.restore(&unit.prepared).unwrap(), unit.original);
    }

    #[test]
    fn sentence_marks_inside_inline_code_are_not_boundaries() {
        assert_eq!(
            translated("Run `a.b! c? 中文。` now. Next.", true),
            ["Run `a.b! c? 中文。` now.", "Next."]
        );
    }

    #[test]
    fn empty_whitespace_and_lone_cr_are_literal() {
        for text in ["", " \t\r\n", "\r\r", "\n\n"] {
            let units = units(text, true);
            assert!(units.iter().all(|unit| !unit.translate));
            assert_eq!(
                units.iter().map(|unit| unit.original).collect::<String>(),
                text
            );
        }
    }

    #[test]
    fn rejects_duplicate_attributes_and_unquoted_ids() {
        let units = units("Use `code`.", true);
        let unit = units.iter().find(|unit| unit.translate).unwrap();
        for response in [
            "<span id='nami-code-0' id='nami-code-0'>x</span>",
            "<span id=nami-code-0>x</span>",
            "<span id='nami-code-0' translate='yes'>x</span>",
        ] {
            assert!(unit.restore(response).is_err());
        }
    }

    #[test]
    fn large_unicode_and_code_input_preserves_all_ranges() {
        let input = "中文。 Run `a < b && c` now!\r\n".repeat(2000);
        let units = units(&input, true);
        assert_eq!(
            units.iter().map(|unit| unit.original).collect::<String>(),
            input
        );
        for unit in units.iter().filter(|unit| unit.translate) {
            assert_eq!(unit.restore(&unit.prepared).unwrap(), unit.original);
        }
    }
}
