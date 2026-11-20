use objc2::{class, msg_send, rc::Retained, runtime::AnyObject};
use objc2_foundation::{ns_string, NSString};

const TRANSIENT_TYPE: &str = "org.nspasteboard.TransientType";

type Item = Vec<(String, Vec<u8>)>;

// All pasteboard access stays on the main thread; snapshots never leave memory.
pub struct CopySnapshot {
    pub version: i64,
    items: Vec<Item>,
    pub initial_text: Option<String>,
    pub initial_has_items: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SelectionRead {
    pub version: i64,
    pub text: Option<String>,
    pub has_items: bool,
}

unsafe fn version(board: *mut AnyObject) -> i64 {
    let count: isize = msg_send![board, changeCount];
    count as i64
}

unsafe fn snapshot(board: *mut AnyObject) -> Result<CopySnapshot, &'static str> {
    let initial = version(board);
    let objects: Option<Retained<AnyObject>> = msg_send![board, pasteboardItems];
    let mut items = Vec::new();
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
    Ok(CopySnapshot {
        version: initial,
        initial_text: read_text(board).filter(|text| !text.is_empty()),
        initial_has_items: !items.is_empty(),
        items,
    })
}

unsafe fn restore(
    board: *mut AnyObject,
    saved: &CopySnapshot,
    transient: bool,
) -> Result<i64, &'static str> {
    // Build and validate every item before clearing anything.
    let objects: Retained<AnyObject> = msg_send![class!(NSMutableArray), array];
    for (index, formats) in saved.items.iter().enumerate() {
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
        if transient && index == 0 && !formats.iter().any(|(kind, _)| kind == TRANSIENT_TYPE) {
            let marker = NSString::from_str(TRANSIENT_TYPE);
            let empty: Retained<AnyObject> = msg_send![class!(NSData), data];
            let success: bool = msg_send![&*item, setData: &*empty, forType: &*marker];
            if !success {
                return Err("clipboard_restore_failed");
            }
        }
        let _: () = msg_send![&*objects, addObject: &*item];
    }
    let _: isize = msg_send![board, clearContents];
    if !saved.items.is_empty() {
        let success: bool = msg_send![board, writeObjects: &*objects];
        if !success {
            return Err("clipboard_restore_failed");
        }
    }
    Ok(version(board))
}

unsafe fn read_text(board: *mut AnyObject) -> Option<String> {
    let text: Option<Retained<NSString>> =
        msg_send![board, stringForType: ns_string!("public.utf8-plain-text")];
    text.map(|text| text.to_string().trim().to_owned())
}

pub fn begin_selection_copy(target: Option<super::Target>) -> Result<CopySnapshot, &'static str> {
    let saved = unsafe { snapshot(super::runtime_pasteboard())? };
    if let Some(target) = target {
        let _ = super::activate(target);
    }
    super::send_copy(target)?;
    Ok(saved)
}

/// Reads the pasteboard state without changing it. `text` is `None` while a
/// copy is still between `clearContents` and its first write, and also for a
/// genuinely non-text payload; `has_items` distinguishes those cases.
pub fn read_selection_payload() -> SelectionRead {
    unsafe {
        let board = super::runtime_pasteboard();
        let version = version(board);
        let items: Option<Retained<AnyObject>> = msg_send![board, pasteboardItems];
        let has_items = items.as_ref().is_some_and(|items| {
            let count: usize = msg_send![items, count];
            count > 0
        });
        SelectionRead {
            version,
            text: read_text(board).filter(|text| !text.is_empty()),
            has_items,
        }
    }
}

unsafe fn finish_on_board(
    board: *mut AnyObject,
    saved: &CopySnapshot,
    text: String,
    observed_version: i64,
) -> Result<(String, i64), &'static str> {
    if version(board) != observed_version {
        return Err("selection_copy_changed");
    }
    let restored_version = restore(board, saved, true)?;
    Ok((text, restored_version))
}

pub fn finish_selection_copy(
    saved: &CopySnapshot,
    text: String,
    observed_version: i64,
) -> Result<(String, i64), &'static str> {
    unsafe { finish_on_board(super::runtime_pasteboard(), saved, text, observed_version) }
}

#[cfg(feature = "ui-tests")]
pub fn test_selection_buffer_text() -> Option<String> {
    unsafe {
        let board = super::runtime_pasteboard();
        let text: Option<Retained<NSString>> =
            msg_send![board, stringForType: ns_string!("public.utf8-plain-text")];
        text.map(|text| text.to_string())
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
            let _: isize = msg_send![board, clearContents];
            let _: bool = msg_send![board, setString: ns_string!("Synthetic text"), forType: ns_string!("public.utf8-plain-text")];
            if read_text(board).as_deref() != Some("Synthetic text") {
                return Err("text_not_read");
            }
            finish_copy(board, &saved, true)?;
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
                initial_text: Some("Original".into()),
                initial_has_items: true,
            };
            restore(board, &original, false)?;
            let saved = snapshot(board)?;
            if saved.items != original.items {
                return Err("multi_item_snapshot_changed");
            }
            // A newer external copy is returned as the selection text and the
            // pre-copy snapshot is restored unconditionally.
            let _: isize = msg_send![board, clearContents];
            let _: bool = msg_send![board, setString: ns_string!("Newer copy"), forType: ns_string!("public.utf8-plain-text")];
            let (text, restored_version) = finish_copy(board, &saved, true)?;
            let mut restored = snapshot(board)?;
            if text != "Newer copy" || restored.version != restored_version {
                return Err("copy_result_or_owned_version_incorrect");
            }
            if !restored.items[0].contains(&(TRANSIENT_TYPE.into(), Vec::new())) {
                return Err("restored_content_not_marked_transient");
            }
            restored.items[0].retain(|(kind, _)| kind != TRANSIENT_TYPE);
            if restored.items != original.items {
                return Err("formats_not_restored");
            }
            let _: isize = msg_send![board, clearContents];
            let _: bool = msg_send![board, setString: ns_string!("  Copied text\n"), forType: ns_string!("public.utf8-plain-text")];
            let (text, _) = finish_copy(board, &saved, true)?;
            if text != "Copied text" {
                return Err("copied_text_not_trimmed");
            }
            Ok::<(), &'static str>(())
        })();
        let _: () = msg_send![board, releaseGlobally];
        result.map_err(str::to_string)
    }
}

#[cfg(feature = "ui-tests")]
unsafe fn finish_copy(
    board: *mut AnyObject,
    saved: &CopySnapshot,
    accept_text: bool,
) -> Result<(String, i64), &'static str> {
    let text = if accept_text { read_text(board) } else { None };
    let observed_version = version(board);
    finish_on_board(board, saved, text.unwrap_or_default(), observed_version)
}
