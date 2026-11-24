use super::{ClipboardTextFormatting, MAX_RICH_TEXT_BYTES};

const MAX_SANITIZED_HTML_BYTES: usize = 256 * 1024;
const MAX_NESTING_DEPTH: usize = 64;

const DROP_SUBTREE_TAGS: &[&str] = &[
    "applet", "audio", "button", "canvas", "form", "frameset", "head", "iframe", "map", "math",
    "noscript", "object", "picture", "script", "select", "style", "svg", "template", "textarea",
    "title", "video",
];

const VOID_ALLOWED_TAGS: &[&str] = &["br", "hr"];

const ALLOWED_TAGS: &[&str] = &[
    "a",
    "b",
    "blockquote",
    "caption",
    "code",
    "dd",
    "del",
    "div",
    "dl",
    "dt",
    "em",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "i",
    "ins",
    "li",
    "mark",
    "ol",
    "p",
    "pre",
    "s",
    "span",
    "strong",
    "sub",
    "sup",
    "table",
    "tbody",
    "td",
    "tfoot",
    "th",
    "thead",
    "tr",
    "u",
    "ul",
];

pub fn format_label(formatting: &ClipboardTextFormatting) -> Option<&'static str> {
    match (formatting.html.is_some(), formatting.rtf.is_some()) {
        (true, true) => Some("html+rtf"),
        (true, false) => Some("html"),
        (false, true) => Some("rtf"),
        (false, false) => None,
    }
}

pub fn preview(formatting: &ClipboardTextFormatting) -> (Option<&'static str>, Option<String>) {
    let label = format_label(formatting);
    let html = formatting
        .html
        .as_deref()
        .and_then(|bytes| {
            if bytes.len() > MAX_RICH_TEXT_BYTES {
                return None;
            }
            let text = String::from_utf8_lossy(bytes);
            sanitize_html(&text)
        })
        .or_else(|| formatting.rtf.as_deref().and_then(rtf_to_safe_html));
    (label, html)
}

pub fn sanitize_html(raw: &str) -> Option<String> {
    if raw.is_empty() || raw.len() > MAX_RICH_TEXT_BYTES {
        return None;
    }
    let bytes = raw.as_bytes();
    let mut index = 0;
    let mut output = String::with_capacity(raw.len().min(4096));
    let mut open_tags: Vec<&'static str> = Vec::new();
    let mut drop_stack: Vec<String> = Vec::new();
    let mut has_visible_text = false;

    while index < bytes.len() && output.len() < MAX_SANITIZED_HTML_BYTES {
        if bytes[index] != b'<' {
            let start = index;
            while index < bytes.len() && bytes[index] != b'<' {
                index += 1;
            }
            if drop_stack.is_empty() {
                let decoded = html_escape::decode_html_entities(&raw[start..index]);
                if decoded.chars().any(|ch| !ch.is_whitespace()) {
                    has_visible_text = true;
                }
                push_bounded(
                    &mut output,
                    &html_escape::encode_text(&decoded),
                    MAX_SANITIZED_HTML_BYTES,
                );
            }
            continue;
        }

        if raw[index..].starts_with("<!--") {
            if let Some(end) = raw[index + 4..].find("-->") {
                index += 4 + end + 3;
            } else {
                break;
            }
            continue;
        }

        if raw[index..].starts_with("<!") || raw[index..].starts_with("<?") {
            if let Some(end) = raw[index + 2..].find('>') {
                index += 2 + end + 1;
            } else {
                break;
            }
            continue;
        }

        let Some((tag_end, tag_body)) = scan_tag(raw, index) else {
            // Bare `<` with no closing `>` — emit escaped literal text when not inside dropped subtree.
            if drop_stack.is_empty() {
                has_visible_text = true;
                push_bounded(&mut output, "&lt;", MAX_SANITIZED_HTML_BYTES);
            }
            index += 1;
            continue;
        };
        index = tag_end;

        let trimmed = tag_body.trim();
        if trimmed.is_empty() {
            continue;
        }

        let is_closing = trimmed.starts_with('/');
        let inner = if is_closing {
            trimmed[1..].trim_start()
        } else {
            trimmed
        };
        let name_end = inner
            .find(|ch: char| ch.is_ascii_whitespace() || ch == '/')
            .unwrap_or(inner.len());
        let tag_name = inner[..name_end].to_ascii_lowercase();
        if tag_name.is_empty()
            || !tag_name
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-')
        {
            continue;
        }

        if is_closing {
            if let Some(pos) = drop_stack.iter().rposition(|name| name == &tag_name) {
                drop_stack.truncate(pos);
                continue;
            }
            if !drop_stack.is_empty() {
                continue;
            }
            if let Some(pos) = open_tags.iter().rposition(|name| *name == tag_name) {
                while open_tags.len() > pos {
                    let popped = open_tags.pop().unwrap();
                    push_bounded(&mut output, "</", MAX_SANITIZED_HTML_BYTES);
                    push_bounded(&mut output, popped, MAX_SANITIZED_HTML_BYTES);
                    push_bounded(&mut output, ">", MAX_SANITIZED_HTML_BYTES);
                }
            }
            continue;
        }

        let self_closing = inner.ends_with('/');
        let attr_str = inner[name_end..].trim_end_matches('/').trim();

        if DROP_SUBTREE_TAGS.contains(&tag_name.as_str()) {
            if !self_closing {
                drop_stack.push(tag_name);
            }
            continue;
        }
        if !drop_stack.is_empty() {
            continue;
        }

        if let Some(&void_tag) = VOID_ALLOWED_TAGS.iter().find(|&&t| t == tag_name) {
            push_bounded(&mut output, "<", MAX_SANITIZED_HTML_BYTES);
            push_bounded(&mut output, void_tag, MAX_SANITIZED_HTML_BYTES);
            push_bounded(&mut output, ">", MAX_SANITIZED_HTML_BYTES);
            continue;
        }

        let Some(&allowed_tag) = ALLOWED_TAGS.iter().find(|&&t| t == tag_name) else {
            continue;
        };

        let attrs = sanitize_attributes(allowed_tag, attr_str);
        if !self_closing && open_tags.len() >= MAX_NESTING_DEPTH {
            continue;
        }

        push_bounded(&mut output, "<", MAX_SANITIZED_HTML_BYTES);
        push_bounded(&mut output, allowed_tag, MAX_SANITIZED_HTML_BYTES);
        for (name, value) in attrs {
            push_bounded(&mut output, " ", MAX_SANITIZED_HTML_BYTES);
            push_bounded(&mut output, name, MAX_SANITIZED_HTML_BYTES);
            push_bounded(&mut output, "=\"", MAX_SANITIZED_HTML_BYTES);
            push_bounded(
                &mut output,
                &html_escape::encode_double_quoted_attribute(&value),
                MAX_SANITIZED_HTML_BYTES,
            );
            push_bounded(&mut output, "\"", MAX_SANITIZED_HTML_BYTES);
        }
        push_bounded(&mut output, ">", MAX_SANITIZED_HTML_BYTES);
        if self_closing {
            push_bounded(&mut output, "</", MAX_SANITIZED_HTML_BYTES);
            push_bounded(&mut output, allowed_tag, MAX_SANITIZED_HTML_BYTES);
            push_bounded(&mut output, ">", MAX_SANITIZED_HTML_BYTES);
        } else {
            open_tags.push(allowed_tag);
        }
    }

    while let Some(popped) = open_tags.pop() {
        output.push_str("</");
        output.push_str(popped);
        output.push('>');
    }

    if has_visible_text {
        Some(output)
    } else {
        None
    }
}

fn push_bounded(target: &mut String, piece: &str, max_bytes: usize) {
    if target.len() >= max_bytes {
        return;
    }
    let remaining = max_bytes - target.len();
    if piece.len() <= remaining {
        target.push_str(piece);
    } else {
        let mut end = remaining;
        while end > 0 && !piece.is_char_boundary(end) {
            end -= 1;
        }
        target.push_str(&piece[..end]);
    }
}

fn scan_tag(raw: &str, start: usize) -> Option<(usize, &str)> {
    let bytes = raw.as_bytes();
    let mut index = start + 1;
    let mut quote: Option<u8> = None;
    while index < bytes.len() {
        let b = bytes[index];
        if let Some(q) = quote {
            if b == q {
                quote = None;
            }
        } else if b == b'"' || b == b'\'' {
            quote = Some(b);
        } else if b == b'>' {
            return Some((index + 1, &raw[start + 1..index]));
        }
        index += 1;
    }
    None
}

fn sanitize_attributes(tag: &str, raw_attrs: &str) -> Vec<(&'static str, String)> {
    let mut result = Vec::new();
    let mut has_href = false;
    let mut has_style = false;
    let mut has_colspan = false;
    let mut has_rowspan = false;

    for (name, value) in parse_attributes(raw_attrs) {
        let decoded = html_escape::decode_html_entities(&value);
        match (tag, name.as_str()) {
            ("a", "href") if !has_href => {
                if let Some(safe_url) = sanitize_href(&decoded) {
                    result.push(("href", safe_url));
                    has_href = true;
                }
            }
            ("td" | "th", "colspan") if !has_colspan => {
                if let Some(span) = parse_table_span(&decoded) {
                    result.push(("colspan", span));
                    has_colspan = true;
                }
            }
            ("td" | "th", "rowspan") if !has_rowspan => {
                if let Some(span) = parse_table_span(&decoded) {
                    result.push(("rowspan", span));
                    has_rowspan = true;
                }
            }
            (_, "style") if !has_style => {
                if let Some(safe_style) = sanitize_style(&decoded) {
                    result.push(("style", safe_style));
                    has_style = true;
                }
            }
            _ => (),
        }
    }

    if tag == "a" && has_href {
        result.push(("rel", "noopener noreferrer".into()));
    }
    result
}

fn parse_attributes(input: &str) -> Vec<(String, String)> {
    let mut attrs = Vec::new();
    let bytes = input.as_bytes();
    let mut i = 0;

    while i < bytes.len() {
        while i < bytes.len() && (bytes[i].is_ascii_whitespace() || bytes[i] == b'/') {
            i += 1;
        }
        if i >= bytes.len() {
            break;
        }
        let name_start = i;
        while i < bytes.len()
            && !bytes[i].is_ascii_whitespace()
            && bytes[i] != b'='
            && bytes[i] != b'/'
        {
            i += 1;
        }
        let name = input[name_start..i].to_ascii_lowercase();
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        let value = if i < bytes.len() && bytes[i] == b'=' {
            i += 1;
            while i < bytes.len() && bytes[i].is_ascii_whitespace() {
                i += 1;
            }
            if i < bytes.len() && (bytes[i] == b'"' || bytes[i] == b'\'') {
                let quote = bytes[i];
                i += 1;
                let val_start = i;
                while i < bytes.len() && bytes[i] != quote {
                    i += 1;
                }
                let val = input[val_start..i].to_string();
                if i < bytes.len() {
                    i += 1;
                }
                val
            } else {
                let val_start = i;
                while i < bytes.len() && !bytes[i].is_ascii_whitespace() {
                    i += 1;
                }
                input[val_start..i].to_string()
            }
        } else {
            String::new()
        };
        if !name.is_empty() {
            attrs.push((name, value));
        }
    }
    attrs
}

fn sanitize_href(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() || trimmed.len() > 4096 {
        return None;
    }
    if trimmed.chars().any(|ch| ch.is_control()) {
        return None;
    }
    let lower = trimmed.to_ascii_lowercase();
    if lower.starts_with("http://") || lower.starts_with("https://") || lower.starts_with("mailto:")
    {
        Some(trimmed.to_string())
    } else {
        None
    }
}

fn parse_table_span(raw: &str) -> Option<String> {
    let value: u32 = raw.trim().parse().ok()?;
    (1..=100).contains(&value).then(|| value.to_string())
}

fn sanitize_style(raw: &str) -> Option<String> {
    let mut parts = Vec::new();
    for declaration in raw.split(';') {
        let Some((prop, value)) = declaration.split_once(':') else {
            continue;
        };
        let prop = prop.trim().to_ascii_lowercase();
        let value = value.trim();
        let value = value.strip_suffix("!important").unwrap_or(value).trim();
        if value.is_empty() || value.len() > 64 {
            continue;
        }
        let lower = value.to_ascii_lowercase();
        if lower.contains("url")
            || lower.contains("expression")
            || lower.contains("import")
            || lower.contains("var(")
            || lower.contains('\\')
            || lower.contains('@')
        {
            continue;
        }
        let safe_value = match prop.as_str() {
            "color" | "background-color" => valid_css_color(&lower).then_some(lower),
            "font-weight" => valid_font_weight(&lower).then_some(lower),
            "font-style" => {
                matches!(lower.as_str(), "normal" | "italic" | "oblique").then_some(lower)
            }
            "text-decoration" => valid_text_decoration(&lower).then_some(lower),
            "text-align" => {
                matches!(lower.as_str(), "left" | "right" | "center" | "justify").then_some(lower)
            }
            _ => None,
        };
        if let Some(safe_value) = safe_value {
            parts.push(format!("{prop}:{safe_value}"));
        }
    }
    (!parts.is_empty()).then(|| parts.join(";"))
}

fn valid_css_color(value: &str) -> bool {
    if let Some(hex) = value.strip_prefix('#') {
        return matches!(hex.len(), 3 | 4 | 6 | 8) && hex.bytes().all(|b| b.is_ascii_hexdigit());
    }
    if let Some(inner) = value
        .strip_prefix("rgb(")
        .or_else(|| value.strip_prefix("rgba("))
        .or_else(|| value.strip_prefix("hsl("))
        .or_else(|| value.strip_prefix("hsla("))
        .and_then(|s| s.strip_suffix(')'))
    {
        return !inner.is_empty()
            && inner.bytes().all(|b| {
                b.is_ascii_digit() || matches!(b, b' ' | b',' | b'.' | b'%' | b'/' | b'+' | b'-')
            });
    }
    matches!(
        value,
        "transparent"
            | "black"
            | "white"
            | "red"
            | "green"
            | "blue"
            | "yellow"
            | "orange"
            | "purple"
            | "pink"
            | "gray"
            | "grey"
            | "silver"
            | "maroon"
            | "navy"
            | "teal"
            | "olive"
            | "aqua"
            | "cyan"
            | "magenta"
            | "fuchsia"
            | "lime"
            | "brown"
            | "gold"
            | "indigo"
            | "violet"
    )
}

fn valid_font_weight(value: &str) -> bool {
    matches!(value, "normal" | "bold" | "bolder" | "lighter")
        || value
            .parse::<u16>()
            .ok()
            .is_some_and(|w| (100..=900).contains(&w) && w % 100 == 0)
}

fn valid_text_decoration(value: &str) -> bool {
    let tokens: Vec<_> = value.split_whitespace().collect();
    !tokens.is_empty()
        && tokens
            .iter()
            .all(|t| matches!(*t, "none" | "underline" | "line-through" | "overline"))
}

#[derive(Clone, Default, PartialEq, Eq)]
struct RtfStyle {
    bold: bool,
    italic: bool,
    underline: bool,
    strike: bool,
    color_index: usize,
    uc: usize,
    skip_group: bool,
    in_colortbl: bool,
}

const RTF_SKIP_DESTINATIONS: &[&str] = &[
    "fonttbl",
    "stylesheet",
    "info",
    "pict",
    "object",
    "header",
    "headerl",
    "headerr",
    "headerf",
    "footer",
    "footerl",
    "footerr",
    "footerf",
    "listtable",
    "listoverridetable",
    "revtbl",
    "rsidtbl",
    "generator",
    "mmathpr",
    "themedata",
    "colorschememapping",
    "datastore",
    "latentstyles",
    "fldinst",
    "xmlnstbl",
    "pgdsctbl",
];

pub fn rtf_to_safe_html(raw: &[u8]) -> Option<String> {
    if raw.is_empty() || raw.len() > MAX_RICH_TEXT_BYTES {
        return None;
    }
    let trimmed_start = raw.iter().position(|b| !b.is_ascii_whitespace())?;
    let bytes = &raw[trimmed_start..];
    if !bytes.starts_with(b"{\\rtf") {
        return None;
    }

    let mut stack: Vec<RtfStyle> = vec![RtfStyle {
        uc: 1,
        ..Default::default()
    }];
    let mut colors: Vec<Option<(u8, u8, u8)>> = Vec::new();
    let mut current_color: (Option<u8>, Option<u8>, Option<u8>) = (None, None, None);
    let mut is_utf8_cpg = false;
    let mut pending_hex: Vec<u8> = Vec::new();
    let mut pending_text = String::new();
    let mut pending_style = RtfStyle::default();
    let mut paragraphs: Vec<String> = vec![String::new()];
    let mut i = 0;

    let flush_text = |pending_text: &mut String,
                      pending_style: &RtfStyle,
                      colors: &[Option<(u8, u8, u8)>],
                      paragraphs: &mut [String]| {
        if pending_text.is_empty() {
            return;
        }
        if let Some(current) = paragraphs.last_mut() {
            append_styled_rtf_text(current, pending_text, pending_style, colors);
        }
        pending_text.clear();
    };

    let push_text = |text: &str,
                     style: &RtfStyle,
                     pending_text: &mut String,
                     pending_style: &mut RtfStyle,
                     colors: &[Option<(u8, u8, u8)>],
                     paragraphs: &mut [String]| {
        if text.is_empty() || style.skip_group || style.in_colortbl {
            return;
        }
        let same_visual = pending_style.bold == style.bold
            && pending_style.italic == style.italic
            && pending_style.underline == style.underline
            && pending_style.strike == style.strike
            && pending_style.color_index == style.color_index;
        if !pending_text.is_empty() && !same_visual {
            flush_text(pending_text, pending_style, colors, paragraphs);
        }
        if pending_text.is_empty() {
            *pending_style = style.clone();
        }
        pending_text.push_str(text);
    };

    let flush_hex = |pending: &mut Vec<u8>,
                     is_utf8: bool,
                     style: &RtfStyle,
                     pending_text: &mut String,
                     pending_style: &mut RtfStyle,
                     colors: &[Option<(u8, u8, u8)>],
                     paragraphs: &mut [String]| {
        if pending.is_empty() {
            return;
        }
        let text = if is_utf8 {
            String::from_utf8_lossy(pending).into_owned()
        } else {
            pending.iter().map(|&b| decode_windows_1252(b)).collect()
        };
        pending.clear();
        push_text(
            &text,
            style,
            pending_text,
            pending_style,
            colors,
            paragraphs,
        );
    };

    while i < bytes.len()
        && paragraphs.iter().map(String::len).sum::<usize>() + pending_text.len()
            < MAX_SANITIZED_HTML_BYTES
    {
        match bytes[i] {
            b'{' => {
                let top = stack.last().cloned().unwrap_or(RtfStyle {
                    uc: 1,
                    ..Default::default()
                });
                flush_hex(
                    &mut pending_hex,
                    is_utf8_cpg,
                    &top,
                    &mut pending_text,
                    &mut pending_style,
                    &colors,
                    &mut paragraphs,
                );
                stack.push(top);
                i += 1;
            }
            b'}' => {
                let top = stack.last().cloned().unwrap_or_default();
                flush_hex(
                    &mut pending_hex,
                    is_utf8_cpg,
                    &top,
                    &mut pending_text,
                    &mut pending_style,
                    &colors,
                    &mut paragraphs,
                );
                if stack.len() > 1 {
                    stack.pop();
                }
                i += 1;
            }
            b'\r' | b'\n' => {
                i += 1;
            }
            b'\\' => {
                i += 1;
                if i >= bytes.len() {
                    break;
                }
                let next = bytes[i];
                if next == b'\'' {
                    if i + 2 < bytes.len() {
                        if let (Some(hi), Some(lo)) =
                            (hex_nibble(bytes[i + 1]), hex_nibble(bytes[i + 2]))
                        {
                            pending_hex.push((hi << 4) | lo);
                            i += 3;
                            continue;
                        }
                    }
                    i += 1;
                    continue;
                }

                let top_snapshot = stack.last().cloned().unwrap_or_default();
                flush_hex(
                    &mut pending_hex,
                    is_utf8_cpg,
                    &top_snapshot,
                    &mut pending_text,
                    &mut pending_style,
                    &colors,
                    &mut paragraphs,
                );

                match next {
                    b'\\' | b'{' | b'}' => {
                        i += 1;
                        if let Some(style) = stack.last() {
                            let s: String = [next as char].into_iter().collect();
                            push_text(
                                &s,
                                style,
                                &mut pending_text,
                                &mut pending_style,
                                &colors,
                                &mut paragraphs,
                            );
                        }
                    }
                    b'*' => {
                        i += 1;
                        if let Some(style) = stack.last_mut() {
                            style.skip_group = true;
                        }
                    }
                    b'~' => {
                        i += 1;
                        if let Some(style) = stack.last() {
                            push_text(
                                "\u{00A0}",
                                style,
                                &mut pending_text,
                                &mut pending_style,
                                &colors,
                                &mut paragraphs,
                            );
                        }
                    }
                    b'_' => {
                        i += 1;
                        if let Some(style) = stack.last() {
                            push_text(
                                "-",
                                style,
                                &mut pending_text,
                                &mut pending_style,
                                &colors,
                                &mut paragraphs,
                            );
                        }
                    }
                    b'\r' | b'\n' => {
                        i += 1;
                        if stack
                            .last()
                            .is_some_and(|s| !s.skip_group && !s.in_colortbl)
                        {
                            flush_text(&mut pending_text, &pending_style, &colors, &mut paragraphs);
                            paragraphs.push(String::new());
                        }
                    }
                    b if b.is_ascii_alphabetic() => {
                        let word_start = i;
                        while i < bytes.len() && bytes[i].is_ascii_alphabetic() {
                            i += 1;
                        }
                        let word = std::str::from_utf8(&bytes[word_start..i])
                            .unwrap_or_default()
                            .to_ascii_lowercase();
                        let mut sign = 1i32;
                        let mut has_param = false;
                        let mut param = 0i32;
                        if i < bytes.len() && bytes[i] == b'-' {
                            sign = -1;
                            i += 1;
                        }
                        let digits_start = i;
                        while i < bytes.len() && bytes[i].is_ascii_digit() {
                            has_param = true;
                            param = param
                                .saturating_mul(10)
                                .saturating_add(i32::from(bytes[i] - b'0'));
                            i += 1;
                        }
                        if has_param {
                            param = param.saturating_mul(sign);
                        } else if digits_start > word_start && sign == -1 {
                            i -= 1;
                        }
                        if i < bytes.len() && bytes[i] == b' ' {
                            i += 1;
                        }

                        if word == "ansicpg" && has_param && param == 65001 {
                            is_utf8_cpg = true;
                            continue;
                        }
                        if word == "colortbl" {
                            if let Some(style) = stack.last_mut() {
                                style.in_colortbl = true;
                            }
                            continue;
                        }
                        if stack.last().is_some_and(|s| s.in_colortbl) {
                            if has_param && (0..=255).contains(&param) {
                                let v = param as u8;
                                match word.as_str() {
                                    "red" => current_color.0 = Some(v),
                                    "green" => current_color.1 = Some(v),
                                    "blue" => current_color.2 = Some(v),
                                    _ => (),
                                }
                            }
                            continue;
                        }
                        if RTF_SKIP_DESTINATIONS.contains(&word.as_str()) {
                            if let Some(style) = stack.last_mut() {
                                style.skip_group = true;
                            }
                            continue;
                        }
                        if stack.last().is_some_and(|s| s.skip_group) {
                            continue;
                        }

                        match word.as_str() {
                            "b" => {
                                if let Some(style) = stack.last_mut() {
                                    style.bold = !has_param || param != 0;
                                }
                            }
                            "i" => {
                                if let Some(style) = stack.last_mut() {
                                    style.italic = !has_param || param != 0;
                                }
                            }
                            "ul" => {
                                if let Some(style) = stack.last_mut() {
                                    style.underline = !has_param || param != 0;
                                }
                            }
                            "ulnone" => {
                                if let Some(style) = stack.last_mut() {
                                    style.underline = false;
                                }
                            }
                            "strike" | "striked" => {
                                if let Some(style) = stack.last_mut() {
                                    style.strike = !has_param || param != 0;
                                }
                            }
                            "plain" => {
                                if let Some(style) = stack.last_mut() {
                                    style.bold = false;
                                    style.italic = false;
                                    style.underline = false;
                                    style.strike = false;
                                    style.color_index = 0;
                                }
                            }
                            "cf" => {
                                if let Some(style) = stack.last_mut() {
                                    style.color_index = if has_param && param >= 0 {
                                        param as usize
                                    } else {
                                        0
                                    };
                                }
                            }
                            "uc" => {
                                if let Some(style) = stack.last_mut() {
                                    if has_param && param >= 0 {
                                        style.uc = (param as usize).min(16);
                                    }
                                }
                            }
                            "u" => {
                                if has_param {
                                    let code_unit = if param < 0 {
                                        (param + 65536) as u16
                                    } else {
                                        param as u16
                                    };
                                    let uc_skip = stack.last().map_or(1, |s| s.uc);
                                    skip_rtf_fallback_chars(bytes, &mut i, uc_skip);
                                    let decoded_char = if (0xD800..=0xDBFF).contains(&code_unit) {
                                        if let Some((low, next_i)) = peek_low_surrogate(bytes, i) {
                                            i = next_i;
                                            skip_rtf_fallback_chars(bytes, &mut i, uc_skip);
                                            let cp = 0x10000
                                                + (((u32::from(code_unit) - 0xD800) << 10)
                                                    | (u32::from(low) - 0xDC00));
                                            char::from_u32(cp)
                                        } else {
                                            None
                                        }
                                    } else {
                                        char::from_u32(u32::from(code_unit))
                                    };
                                    if let (Some(ch), Some(style)) = (decoded_char, stack.last()) {
                                        let s = ch.to_string();
                                        push_text(
                                            &s,
                                            style,
                                            &mut pending_text,
                                            &mut pending_style,
                                            &colors,
                                            &mut paragraphs,
                                        );
                                    }
                                }
                            }
                            "par" => {
                                flush_text(
                                    &mut pending_text,
                                    &pending_style,
                                    &colors,
                                    &mut paragraphs,
                                );
                                paragraphs.push(String::new());
                            }
                            "line" => {
                                flush_text(
                                    &mut pending_text,
                                    &pending_style,
                                    &colors,
                                    &mut paragraphs,
                                );
                                if let Some(current) = paragraphs.last_mut() {
                                    current.push_str("<br>");
                                }
                            }
                            "tab" => {
                                if let Some(style) = stack.last() {
                                    push_text(
                                        "\t",
                                        style,
                                        &mut pending_text,
                                        &mut pending_style,
                                        &colors,
                                        &mut paragraphs,
                                    );
                                }
                            }
                            "emdash" | "endash" | "lquote" | "rquote" | "ldblquote"
                            | "rdblquote" | "bullet" => {
                                let symbol = match word.as_str() {
                                    "emdash" => "\u{2014}",
                                    "endash" => "\u{2013}",
                                    "lquote" => "\u{2018}",
                                    "rquote" => "\u{2019}",
                                    "ldblquote" => "\u{201C}",
                                    "rdblquote" => "\u{201D}",
                                    "bullet" => "\u{2022}",
                                    _ => "",
                                };
                                if let Some(style) = stack.last() {
                                    push_text(
                                        symbol,
                                        style,
                                        &mut pending_text,
                                        &mut pending_style,
                                        &colors,
                                        &mut paragraphs,
                                    );
                                }
                            }
                            _ => (),
                        }
                    }
                    _ => {
                        i += 1;
                    }
                }
            }
            b';' if stack.last().is_some_and(|s| s.in_colortbl) => {
                let entry = match current_color {
                    (Some(r), Some(g), Some(b)) => Some((r, g, b)),
                    _ => None,
                };
                colors.push(entry);
                current_color = (None, None, None);
                i += 1;
            }
            other => {
                let top = stack.last().cloned().unwrap_or_default();
                flush_hex(
                    &mut pending_hex,
                    is_utf8_cpg,
                    &top,
                    &mut pending_text,
                    &mut pending_style,
                    &colors,
                    &mut paragraphs,
                );
                if !top.skip_group && !top.in_colortbl {
                    let mut run = String::new();
                    run.push(decode_windows_1252(other));
                    i += 1;
                    while i < bytes.len()
                        && !matches!(bytes[i], b'{' | b'}' | b'\\' | b'\r' | b'\n')
                    {
                        run.push(decode_windows_1252(bytes[i]));
                        i += 1;
                    }
                    push_text(
                        &run,
                        &top,
                        &mut pending_text,
                        &mut pending_style,
                        &colors,
                        &mut paragraphs,
                    );
                } else {
                    i += 1;
                }
            }
        }
    }

    if let Some(top) = stack.last() {
        flush_hex(
            &mut pending_hex,
            is_utf8_cpg,
            top,
            &mut pending_text,
            &mut pending_style,
            &colors,
            &mut paragraphs,
        );
    }
    flush_text(&mut pending_text, &pending_style, &colors, &mut paragraphs);

    while paragraphs.len() > 1 && paragraphs.last().is_some_and(String::is_empty) {
        paragraphs.pop();
    }

    let mut html = String::new();
    for para in paragraphs {
        if para.is_empty() {
            html.push_str("<p><br></p>");
        } else {
            html.push_str("<p>");
            html.push_str(&para);
            html.push_str("</p>");
        }
    }
    sanitize_html(&html)
}

fn append_styled_rtf_text(
    target: &mut String,
    text: &str,
    style: &RtfStyle,
    colors: &[Option<(u8, u8, u8)>],
) {
    let escaped = html_escape::encode_text(text);
    let color = style
        .color_index
        .checked_sub(0)
        .and_then(|idx| colors.get(idx).copied().flatten());
    if let Some((r, g, b)) = color {
        target.push_str(&format!("<span style=\"color:rgb({r},{g},{b})\">"));
    }
    if style.bold {
        target.push_str("<strong>");
    }
    if style.italic {
        target.push_str("<em>");
    }
    if style.underline {
        target.push_str("<u>");
    }
    if style.strike {
        target.push_str("<s>");
    }
    target.push_str(&escaped);
    if style.strike {
        target.push_str("</s>");
    }
    if style.underline {
        target.push_str("</u>");
    }
    if style.italic {
        target.push_str("</em>");
    }
    if style.bold {
        target.push_str("</strong>");
    }
    if color.is_some() {
        target.push_str("</span>");
    }
}

fn skip_rtf_fallback_chars(bytes: &[u8], i: &mut usize, count: usize) {
    for _ in 0..count {
        if *i >= bytes.len() || bytes[*i] == b'{' || bytes[*i] == b'}' {
            break;
        }
        if bytes[*i] == b'\\' && *i + 4 <= bytes.len() && bytes[*i + 1] == b'\'' {
            *i += 4;
        } else {
            *i += 1;
        }
    }
}

fn peek_low_surrogate(bytes: &[u8], start: usize) -> Option<(u16, usize)> {
    if start + 2 >= bytes.len() || bytes[start] != b'\\' || bytes[start + 1] != b'u' {
        return None;
    }
    let mut i = start + 2;
    let mut sign = 1i32;
    if i < bytes.len() && bytes[i] == b'-' {
        sign = -1;
        i += 1;
    }
    let digits_start = i;
    let mut param = 0i32;
    while i < bytes.len() && bytes[i].is_ascii_digit() {
        param = param
            .saturating_mul(10)
            .saturating_add(i32::from(bytes[i] - b'0'));
        i += 1;
    }
    if i == digits_start {
        return None;
    }
    param = param.saturating_mul(sign);
    if i < bytes.len() && bytes[i] == b' ' {
        i += 1;
    }
    let unit = if param < 0 {
        (param + 65536) as u16
    } else {
        param as u16
    };
    (0xDC00..=0xDFFF).contains(&unit).then_some((unit, i))
}

fn hex_nibble(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

fn decode_windows_1252(byte: u8) -> char {
    match byte {
        0x80 => '\u{20AC}',
        0x82 => '\u{201A}',
        0x83 => '\u{0192}',
        0x84 => '\u{201E}',
        0x85 => '\u{2026}',
        0x86 => '\u{2020}',
        0x87 => '\u{2021}',
        0x88 => '\u{02C6}',
        0x89 => '\u{2030}',
        0x8A => '\u{0160}',
        0x8B => '\u{2039}',
        0x8C => '\u{0152}',
        0x8E => '\u{017D}',
        0x91 => '\u{2018}',
        0x92 => '\u{2019}',
        0x93 => '\u{201C}',
        0x94 => '\u{201D}',
        0x95 => '\u{2022}',
        0x96 => '\u{2013}',
        0x97 => '\u{2014}',
        0x98 => '\u{02DC}',
        0x99 => '\u{2122}',
        0x9A => '\u{0161}',
        0x9B => '\u{203A}',
        0x9C => '\u{0153}',
        0x9E => '\u{017E}',
        0x9F => '\u{0178}',
        _ => char::from(byte),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_html_strips_scripts_events_external_resources_and_preserves_safe_formatting() {
        let dirty = r#"
            <html><head><style>body{background:url(https://evil.example/a.png)}</style><title>Secret</title></head>
            <body>
                <script>alert(1)</script>
                <p onclick="alert(2)" style="color: #ff00aa; background-image: url(https://evil.example/x); font-weight: bold">
                    Hello <strong>World</strong>
                </p>
                <img src="https://tracker.example/pixel.png" onerror="alert(3)">
                <a href="javascript:alert(4)">Bad Link</a>
                <a href="https://example.com/docs" target="_blank">Good Link</a>
                <iframe src="https://evil.example"></iframe>
                <svg><script>alert(5)</script></svg>
            </body></html>
        "#;
        let clean = sanitize_html(dirty).unwrap();
        assert!(!clean.contains("script"));
        assert!(!clean.contains("alert("));
        assert!(!clean.contains("onclick"));
        assert!(!clean.contains("onerror"));
        assert!(!clean.contains("<img"));
        assert!(!clean.contains("<iframe"));
        assert!(!clean.contains("<svg"));
        assert!(!clean.contains("tracker.example"));
        assert!(!clean.contains("evil.example"));
        assert!(!clean.contains("javascript:"));
        assert!(!clean.contains("Secret"));
        assert!(clean.contains("<strong>World</strong>"));
        assert!(clean.contains("color:#ff00aa;font-weight:bold"));
        assert!(clean.contains(r#"href="https://example.com/docs""#));
        assert!(clean.contains(r#"rel="noopener noreferrer""#));
        assert!(clean.contains("Bad Link"));
    }

    #[test]
    fn sanitize_html_closes_unbalanced_tags_and_rejects_empty_markup() {
        let unbalanced = "<div><p><strong>Text";
        assert_eq!(
            sanitize_html(unbalanced).as_deref(),
            Some("<div><p><strong>Text</strong></p></div>")
        );
        assert_eq!(sanitize_html("<script>alert(1)</script><img src=x>"), None);
    }

    #[test]
    fn rtf_to_safe_html_renders_formatting_colors_and_unicode() {
        let rtf = br#"{\rtf1\ansi\deff0{\fonttbl{\f0 Helvetica;}}{\colortbl;\red255\green0\blue0;}\b Bold \i Italic \cf1 Red\cf0\par \u20013?\u25991? \u-10180?\u-8438?}"#;
        let html = rtf_to_safe_html(rtf).unwrap();
        assert!(html.contains("<strong>Bold </strong>"));
        assert!(html.contains("<em>Italic </em>"));
        assert!(html.contains("color:rgb(255,0,0)"));
        assert!(html.contains("中文"));
        assert!(html.contains("🌊"));
        assert!(!html.contains("Helvetica"));
    }
}
