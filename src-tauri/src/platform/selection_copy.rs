use objc2::{class, msg_send, rc::Retained, runtime::AnyObject};
use objc2_foundation::{ns_string, NSString};

type Item = Vec<(String, Vec<u8>)>;

// All pasteboard access stays on the main thread; snapshots never leave memory.
pub struct CopySnapshot {
    pub version: i64,
    items: Vec<Item>,
}

unsafe fn version(board: *mut AnyObject) -> i64 {
    let count: isize = msg_send![board, changeCount];
    count as i64
}

unsafe fn snapshot(board: *mut AnyObject) -> Result<CopySnapshot, &'static str> {
    let initial = version(board);
    let objects: Option<Retained<AnyObject>> = msg_send![board, pasteboardItems];
    let mut items = Vec::new();
    let mut total = 0usize;
    if let Some(objects) = objects {
        let count: usize = msg_send![&*objects, count];
        for index in 0..count {
            let item: *mut AnyObject = msg_send![&*objects, objectAtIndex: index];
            let types: Retained<AnyObject> = msg_send![item, types];
            let count: usize = msg_send![&*types, count];
            let mut formats = Vec::new();
            for index in 0..count {
                let kind: Retained<NSString> = msg_send![&*types, objectAtIndex: index];
                let data: Option<Retained<AnyObject>> = msg_send![item, dataForType: &*kind];
                let data = data.ok_or("clipboard_snapshot_unavailable")?;
                let length: usize = msg_send![&*data, length];
                total = total
                    .checked_add(length)
                    .ok_or("clipboard_snapshot_limit")?;
                if total > 64 * 1024 * 1024 {
                    return Err("clipboard_snapshot_limit");
                }
                let bytes: *const u8 = msg_send![&*data, bytes];
                let bytes = if length == 0 {
                    Vec::new()
                } else {
                    std::slice::from_raw_parts(bytes, length).to_vec()
                };
                formats.push((kind.to_string(), bytes));
            }
            items.push(formats);
        }
    }
    if version(board) != initial {
        return Err("clipboard_changed");
    }
    Ok(CopySnapshot {
        version: initial,
        items,
    })
}

unsafe fn restore(
    board: *mut AnyObject,
    saved: &CopySnapshot,
    expected: i64,
) -> Result<(), &'static str> {
    // Build and validate every item before clearing anything.
    let objects: Retained<AnyObject> = msg_send![class!(NSMutableArray), array];
    for formats in &saved.items {
        let item: Retained<AnyObject> = msg_send![class!(NSPasteboardItem), new];
        for (kind, bytes) in formats {
            let kind = NSString::from_str(kind);
            let data: Retained<AnyObject> =
                msg_send![class!(NSData), dataWithBytes: bytes.as_ptr(), length: bytes.len()];
            let success: bool = msg_send![&*item, setData: &*data, forType: &*kind];
            if !success {
                return Err("clipboard_restore_failed");
            }
        }
        let _: () = msg_send![&*objects, addObject: &*item];
    }
    if version(board) != expected {
        return Err("clipboard_changed");
    }
    let owned: isize = msg_send![board, prepareForNewContentsWithOptions: 1usize];
    if version(board) != owned as i64 {
        return Err("clipboard_changed");
    }
    if !saved.items.is_empty() {
        let success: bool = msg_send![board, writeObjects: &*objects];
        if !success {
            return Err("clipboard_restore_failed");
        }
    }
    Ok(())
}

fn non_text_type(kind: &str) -> bool {
    super::clipboard::PRIVATE_TYPES.contains(&kind)
        || matches!(
            kind,
            "public.file-url"
                | "NSFilenamesPboardType"
                | "public.image"
                | "public.png"
                | "public.tiff"
                | "public.jpeg"
                | "public.heic"
                | "com.adobe.pdf"
                | "com.apple.pict"
                | "public.movie"
                | "public.audio"
                | "public.audiovisual-content"
                | "public.vcard"
        )
        || kind.contains("filepromise")
        || kind.contains("promised-file")
        || kind.contains("file-url")
}

unsafe fn read_text(board: *mut AnyObject) -> Option<String> {
    let objects: Option<Retained<AnyObject>> = msg_send![board, pasteboardItems];
    let objects = objects?;
    let count: usize = msg_send![&*objects, count];
    if count != 1 {
        return None;
    }
    let item: *mut AnyObject = msg_send![&*objects, objectAtIndex: 0usize];
    let types: Retained<AnyObject> = msg_send![item, types];
    let count: usize = msg_send![&*types, count];
    for index in 0..count {
        let kind: Retained<NSString> = msg_send![&*types, objectAtIndex: index];
        if non_text_type(&kind.to_string()) {
            return None;
        }
        let workspace: *mut AnyObject = msg_send![class!(NSWorkspace), sharedWorkspace];
        for base in [
            "public.image",
            "public.audiovisual-content",
            "public.archive",
            "public.executable",
            "public.directory",
        ] {
            let base = NSString::from_str(base);
            let non_text: bool = msg_send![workspace, type: &*kind, conformsToType: &*base];
            if non_text {
                return None;
            }
        }
    }
    let text: Option<Retained<NSString>> =
        msg_send![item, stringForType: ns_string!("public.utf8-plain-text")];
    text.map(|text| text.to_string())
        .filter(|text| !text.trim().is_empty())
}

pub fn begin_selection_copy(target: super::Target) -> Result<CopySnapshot, &'static str> {
    if !super::target_focused(target) {
        return Err("target_unavailable");
    }
    if super::secure_input() {
        return Err("selection_secure");
    }
    let saved = unsafe { snapshot(super::runtime_pasteboard())? };
    super::send_copy(target)?;
    Ok(saved)
}

pub fn finish_selection_copy(
    saved: &CopySnapshot,
    expected: i64,
    accept_text: bool,
) -> Result<String, &'static str> {
    unsafe {
        let board = super::runtime_pasteboard();
        if expected == saved.version || version(board) != expected {
            return Err("clipboard_changed");
        }
        let text = if accept_text { read_text(board) } else { None };
        restore(board, saved, expected)?;
        Ok(text.unwrap_or_default())
    }
}

#[cfg(all(feature = "ui-tests", not(feature = "portable-ui")))]
pub fn test_selection_clipboard() -> Result<(), String> {
    unsafe {
        let name = NSString::from_str(&format!(
            "dev.local.nami.ui-tests.selection.{}",
            std::process::id()
        ));
        let board: *mut AnyObject = msg_send![class!(NSPasteboard), pasteboardWithName: &*name];
        let result = (|| {
            let _: isize = msg_send![board, clearContents];
            let saved = snapshot(board)?;
            let _: bool = msg_send![board, setString: ns_string!("Synthetic text"), forType: ns_string!("public.utf8-plain-text")];
            if read_text(board).as_deref() != Some("Synthetic text") {
                return Err("text_not_read");
            }
            restore(board, &saved, version(board))?;
            if !snapshot(board)?.items.is_empty() {
                return Err("empty_clipboard_not_restored");
            }
            let original = CopySnapshot {
                version: version(board),
                items: vec![
                    vec![
                        ("public.utf8-plain-text".into(), b"Original".to_vec()),
                        ("public.html".into(), b"<b>Original</b>".to_vec()),
                    ],
                    vec![("dev.local.synthetic".into(), vec![0, 1, 2, 255])],
                ],
            };
            restore(board, &original, version(board))?;
            let saved = snapshot(board)?;
            if saved.items != original.items {
                return Err("multi_item_snapshot_changed");
            }
            let _: isize = msg_send![board, clearContents];
            let _: bool = msg_send![board, setString: ns_string!("Synthetic selection"), forType: ns_string!("public.utf8-plain-text")];
            let expected = version(board);
            let _: isize = msg_send![board, clearContents];
            let _: bool = msg_send![board, setString: ns_string!("Newer copy"), forType: ns_string!("public.utf8-plain-text")];
            if restore(board, &saved, expected) != Err("clipboard_changed")
                || read_text(board).as_deref() != Some("Newer copy")
            {
                return Err("external_copy_overwritten");
            }
            restore(board, &saved, version(board))?;
            if snapshot(board)?.items != original.items {
                return Err("formats_not_restored");
            }
            for kind in [
                "public.file-url",
                "public.png",
                "com.adobe.pdf",
                "public.movie",
                "public.audio",
                "org.nspasteboard.ConcealedType",
                "org.nspasteboard.TransientType",
                "com.agilebits.onepassword",
            ] {
                let payload = CopySnapshot {
                    version: version(board),
                    items: vec![vec![
                        (
                            "public.utf8-plain-text".into(),
                            b"not selected text".to_vec(),
                        ),
                        (kind.into(), b"synthetic".to_vec()),
                    ]],
                };
                restore(board, &payload, version(board))?;
                if read_text(board).is_some() {
                    return Err("non_text_accepted");
                }
            }
            Ok::<(), &'static str>(())
        })();
        let _: () = msg_send![board, releaseGlobally];
        result.map_err(str::to_string)
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn file_and_image_representations_take_precedence_over_text() {
        for kind in [
            "public.file-url",
            "NSFilenamesPboardType",
            "public.png",
            "public.jpeg",
            "com.adobe.pdf",
            "com.apple.pasteboard.promised-file-url",
            "com.apple.filepromise",
            "org.nspasteboard.ConcealedType",
            "org.nspasteboard.TransientType",
            "com.agilebits.onepassword",
        ] {
            assert!(super::non_text_type(kind));
        }
        for kind in [
            "public.utf8-plain-text",
            "public.html",
            "public.rtf",
            "public.url",
        ] {
            assert!(!super::non_text_type(kind));
        }
    }
}
