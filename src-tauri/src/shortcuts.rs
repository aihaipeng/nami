use serde::{Deserialize, Serialize};

pub const CLIPBOARD_SHORTCUT_CODE: &str = "Backquote";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ShortcutAction {
    Selection,
    Translate,
    CopyPrimary,
    CopySecondary,
    Pin,
    Actions,
    ShowShortcuts,
    FocusSearch,
}

impl ShortcutAction {
    pub fn parse(value: &str) -> Result<Self, String> {
        Ok(match value {
            "selection" => Self::Selection,
            "translate" => Self::Translate,
            "copyPrimary" => Self::CopyPrimary,
            "copySecondary" => Self::CopySecondary,
            "pin" => Self::Pin,
            "actions" => Self::Actions,
            "showShortcuts" => Self::ShowShortcuts,
            "focusSearch" => Self::FocusSearch,
            _ => return Err("invalid_argument".into()),
        })
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Selection => "selection",
            Self::Translate => "translate",
            Self::CopyPrimary => "copyPrimary",
            Self::CopySecondary => "copySecondary",
            Self::Pin => "pin",
            Self::Actions => "actions",
            Self::ShowShortcuts => "showShortcuts",
            Self::FocusSearch => "focusSearch",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KeyBinding {
    pub code: String,
    pub modifiers: Vec<String>,
}

impl KeyBinding {
    pub fn new(code: &str, modifiers: &[&str]) -> Self {
        Self {
            code: code.into(),
            modifiers: modifiers
                .iter()
                .map(|modifier| (*modifier).to_string())
                .collect(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TranslationShortcuts {
    pub selection: Option<KeyBinding>,
    pub translate: Option<KeyBinding>,
    pub copy_primary: Option<KeyBinding>,
    pub copy_secondary: Option<KeyBinding>,
    pub pin: Option<KeyBinding>,
    pub actions: Option<KeyBinding>,
    #[serde(default)]
    pub show_shortcuts: Option<KeyBinding>,
    #[serde(default)]
    pub focus_search: Option<KeyBinding>,
}

impl TranslationShortcuts {
    pub fn defaults(primary: &str) -> Self {
        Self {
            selection: Some(KeyBinding::new("KeyT", &[primary])),
            translate: Some(KeyBinding::new("Enter", &[primary])),
            copy_primary: Some(KeyBinding::new("Digit1", &[primary])),
            copy_secondary: Some(KeyBinding::new("Digit2", &[primary])),
            pin: Some(KeyBinding::new("KeyP", &[primary, "shift"])),
            actions: Some(KeyBinding::new("KeyK", &[primary])),
            show_shortcuts: Some(KeyBinding::new("Slash", &[primary])),
            focus_search: Some(KeyBinding::new("KeyF", &[primary])),
        }
    }

    pub fn from_store(read: impl Fn(&str) -> Option<String>, primary: &str) -> Self {
        let defaults = Self::defaults(primary);
        let stored = read("translation_shortcuts")
            .and_then(|value| serde_json::from_str::<serde_json::Value>(&value).ok())
            .and_then(|value| {
                let shortcuts = Self::deserialize(&value).ok()?;
                Some((value, shortcuts))
            });
        let (keys, mut shortcuts) =
            stored.unwrap_or_else(|| (serde_json::Value::Null, defaults.clone()));
        let clipboard = read("clipboard_shortcut")
            .and_then(|value| serde_json::from_str::<Option<KeyBinding>>(&value).ok())
            .unwrap_or_else(|| Some(clipboard_binding()));
        let records = ClipboardRecordShortcuts::from_store(read("clipboard_record_shortcuts"));
        for action in [ShortcutAction::ShowShortcuts, ShortcutAction::FocusSearch] {
            if keys.get(action.as_str()).is_none() {
                let binding = defaults.get(action).cloned().filter(|binding| {
                    !records.conflicts(binding, None)
                        && !clipboard
                            .as_ref()
                            .is_some_and(|current| same_binding(current, binding))
                        && translation_conflict(&shortcuts, Some(action), binding).is_ok()
                });
                shortcuts.set(action, binding);
            }
        }
        shortcuts
    }

    pub fn general_bindings(&self) -> impl Iterator<Item = &KeyBinding> {
        GENERAL_SHORTCUT_ACTIONS
            .into_iter()
            .filter_map(|action| self.get(action))
    }

    pub fn get(&self, action: ShortcutAction) -> Option<&KeyBinding> {
        match action {
            ShortcutAction::Selection => self.selection.as_ref(),
            ShortcutAction::Translate => self.translate.as_ref(),
            ShortcutAction::CopyPrimary => self.copy_primary.as_ref(),
            ShortcutAction::CopySecondary => self.copy_secondary.as_ref(),
            ShortcutAction::Pin => self.pin.as_ref(),
            ShortcutAction::Actions => self.actions.as_ref(),
            ShortcutAction::ShowShortcuts => self.show_shortcuts.as_ref(),
            ShortcutAction::FocusSearch => self.focus_search.as_ref(),
        }
    }

    pub fn set(&mut self, action: ShortcutAction, binding: Option<KeyBinding>) {
        let slot = match action {
            ShortcutAction::Selection => &mut self.selection,
            ShortcutAction::Translate => &mut self.translate,
            ShortcutAction::CopyPrimary => &mut self.copy_primary,
            ShortcutAction::CopySecondary => &mut self.copy_secondary,
            ShortcutAction::Pin => &mut self.pin,
            ShortcutAction::Actions => &mut self.actions,
            ShortcutAction::ShowShortcuts => &mut self.show_shortcuts,
            ShortcutAction::FocusSearch => &mut self.focus_search,
        };
        *slot = binding;
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ClipboardRecordAction {
    Preview,
    Note,
    Ocr,
    Translate,
    Favorite,
    Pin,
    Delete,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct RecordShortcut {
    pub binding: Option<KeyBinding>,
    pub enabled: bool,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ClipboardRecordShortcuts {
    preview: RecordShortcut,
    note: RecordShortcut,
    ocr: RecordShortcut,
    translate: RecordShortcut,
    favorite: RecordShortcut,
    pin: RecordShortcut,
    delete: RecordShortcut,
}

impl Default for ClipboardRecordShortcuts {
    fn default() -> Self {
        let enabled = |code, modifiers: &[&str]| RecordShortcut {
            binding: Some(KeyBinding::new(code, modifiers)),
            enabled: true,
        };
        let primary = if cfg!(target_os = "macos") {
            "meta"
        } else {
            "control"
        };
        let shifted = if primary == "meta" {
            ["shift", "meta"]
        } else {
            ["control", "shift"]
        };
        Self {
            preview: enabled("KeyY", &[primary]),
            note: enabled("KeyN", &shifted),
            ocr: enabled("KeyO", &shifted),
            translate: enabled("KeyT", &shifted),
            favorite: enabled("KeyF", &["alt"]),
            pin: enabled("KeyP", &["alt"]),
            delete: enabled("Backspace", &[primary]),
        }
    }
}

impl ClipboardRecordShortcuts {
    fn entries(&self) -> [(ClipboardRecordAction, &RecordShortcut); 7] {
        use ClipboardRecordAction::*;
        [
            (Preview, &self.preview),
            (Note, &self.note),
            (Ocr, &self.ocr),
            (Translate, &self.translate),
            (Favorite, &self.favorite),
            (Pin, &self.pin),
            (Delete, &self.delete),
        ]
    }

    pub fn conflicts(&self, binding: &KeyBinding, except: Option<ClipboardRecordAction>) -> bool {
        self.entries().into_iter().any(|(action, current)| {
            Some(action) != except
                && current
                    .binding
                    .as_ref()
                    .is_some_and(|current| same_binding(current, binding))
        })
    }

    pub fn set(
        &mut self,
        action: ClipboardRecordAction,
        binding: Option<KeyBinding>,
        enabled: Option<bool>,
        globals: &[Option<&KeyBinding>],
    ) -> Result<(), String> {
        if binding.is_some() && enabled.is_some() {
            return Err("invalid_argument".into());
        }
        let next = binding.map(checked_record_binding).transpose()?;
        let current = || {
            self.entries().into_iter().find_map(|(entry, value)| {
                (entry == action)
                    .then_some(value.binding.as_ref())
                    .flatten()
            })
        };
        if let Some(value) = next
            .as_ref()
            .or_else(|| enabled.filter(|value| *value).and_then(|_| current()))
        {
            if self.conflicts(value, Some(action))
                || globals
                    .iter()
                    .flatten()
                    .any(|global| same_binding(value, global))
            {
                return Err("shortcut_clipboard_panel".into());
            }
        }
        let current = match action {
            ClipboardRecordAction::Preview => &mut self.preview,
            ClipboardRecordAction::Note => &mut self.note,
            ClipboardRecordAction::Ocr => &mut self.ocr,
            ClipboardRecordAction::Translate => &mut self.translate,
            ClipboardRecordAction::Favorite => &mut self.favorite,
            ClipboardRecordAction::Pin => &mut self.pin,
            ClipboardRecordAction::Delete => &mut self.delete,
        };
        if let Some(enabled) = enabled {
            current.enabled = enabled;
        } else {
            current.binding = next;
        }
        Ok(())
    }

    pub fn from_store(value: Option<String>) -> Self {
        value
            .and_then(|value| {
                let value: serde_json::Value = serde_json::from_str(&value).ok()?;
                let mut shortcuts = serde_json::from_value::<Self>(value.clone()).ok()?;
                // Newly added defaults yield to existing assignments on upgrade.
                for (key, slot) in [
                    ("note", &mut shortcuts.note),
                    ("ocr", &mut shortcuts.ocr),
                    ("translate", &mut shortcuts.translate),
                ] {
                    if value.get(key).is_none()
                        && slot.binding.as_ref().is_some_and(|binding| {
                            [
                                &shortcuts.preview,
                                &shortcuts.favorite,
                                &shortcuts.pin,
                                &shortcuts.delete,
                            ]
                            .into_iter()
                            .any(|current| {
                                current
                                    .binding
                                    .as_ref()
                                    .is_some_and(|current| same_binding(binding, current))
                            })
                        })
                    {
                        slot.binding = None;
                    }
                }
                Some(shortcuts)
            })
            .map(|mut shortcuts| {
                let legacy_preview = shortcuts
                    .preview
                    .binding
                    .as_ref()
                    .is_some_and(|binding| binding.modifiers.is_empty());
                for shortcut in [
                    &mut shortcuts.preview,
                    &mut shortcuts.note,
                    &mut shortcuts.ocr,
                    &mut shortcuts.translate,
                    &mut shortcuts.favorite,
                    &mut shortcuts.pin,
                    &mut shortcuts.delete,
                ] {
                    if shortcut
                        .binding
                        .as_ref()
                        .is_some_and(|binding| binding.modifiers.is_empty())
                    {
                        shortcut.binding = None;
                    }
                }
                if shortcuts.delete.binding.as_ref().is_some_and(|binding| {
                    same_binding(binding, &KeyBinding::new("Backspace", &["alt"]))
                }) {
                    shortcuts.delete.binding = Self::default()
                        .delete
                        .binding
                        .filter(|binding| {
                            !shortcuts.conflicts(binding, Some(ClipboardRecordAction::Delete))
                        })
                        .or(shortcuts.delete.binding);
                }
                if legacy_preview {
                    shortcuts.preview.binding = Self::default()
                        .preview
                        .binding
                        .filter(|binding| !shortcuts.conflicts(binding, None));
                }
                shortcuts
            })
            .filter(|shortcuts| {
                shortcuts.entries().into_iter().all(|(action, current)| {
                    current.binding.as_ref().is_none_or(|binding| {
                        checked_record_binding(binding.clone()).is_ok()
                            && !shortcuts.conflicts(binding, Some(action))
                    })
                })
            })
            .unwrap_or_default()
    }
}

fn checked_record_binding(binding: KeyBinding) -> Result<KeyBinding, String> {
    let binding = normalize_key(binding, true)?;
    if reserved(&binding) {
        return Err("shortcut_reserved".into());
    }
    Ok(binding)
}

pub fn clipboard_binding() -> KeyBinding {
    KeyBinding::new(CLIPBOARD_SHORTCUT_CODE, &["control"])
}

pub fn normalize_binding(binding: KeyBinding) -> Result<KeyBinding, String> {
    normalize_key(binding, false)
}

fn normalize_key(binding: KeyBinding, panel: bool) -> Result<KeyBinding, String> {
    let code = if binding.code == "NumpadEnter" {
        "Enter".to_string()
    } else {
        binding.code
    };
    if !(supported_code(&code) || panel && code == "Backspace") {
        return Err("shortcut_key".into());
    }
    let mut modifiers = Vec::new();
    for name in ["control", "alt", "shift", "meta"] {
        if binding.modifiers.iter().any(|modifier| modifier == name) {
            modifiers.push(name.to_string());
        }
    }
    let modified = modifiers
        .iter()
        .any(|modifier| matches!(modifier.as_str(), "meta" | "control" | "alt"));
    if modifiers.len() != binding.modifiers.len() || !modified {
        return Err("shortcut_modifier".into());
    }
    Ok(KeyBinding { code, modifiers })
}

const TRANSLATION_ACTIONS: [ShortcutAction; 8] = [
    ShortcutAction::Selection,
    ShortcutAction::Translate,
    ShortcutAction::CopyPrimary,
    ShortcutAction::CopySecondary,
    ShortcutAction::Pin,
    ShortcutAction::Actions,
    ShortcutAction::ShowShortcuts,
    ShortcutAction::FocusSearch,
];

pub const GENERAL_SHORTCUT_ACTIONS: [ShortcutAction; 4] = [
    ShortcutAction::Selection,
    ShortcutAction::Pin,
    ShortcutAction::ShowShortcuts,
    ShortcutAction::FocusSearch,
];

fn checked_binding(binding: KeyBinding) -> Result<KeyBinding, String> {
    let binding = normalize_binding(binding)?;
    if reserved(&binding) {
        return Err("shortcut_reserved".into());
    }
    Ok(binding)
}

fn translation_conflict(
    shortcuts: &TranslationShortcuts,
    except: Option<ShortcutAction>,
    binding: &KeyBinding,
) -> Result<(), String> {
    for action in TRANSLATION_ACTIONS {
        if Some(action) != except
            && shortcuts
                .get(action)
                .is_some_and(|current| same_binding(current, binding))
        {
            return Err(format!("shortcut_conflict:{}", action.as_str()));
        }
    }
    Ok(())
}

pub fn validate_clipboard_binding(
    binding: KeyBinding,
    shortcuts: &TranslationShortcuts,
) -> Result<KeyBinding, String> {
    let binding = checked_binding(binding)?;
    translation_conflict(shortcuts, None, &binding)?;
    Ok(binding)
}

pub fn assign_shortcut(
    shortcuts: &TranslationShortcuts,
    action: ShortcutAction,
    binding: Option<KeyBinding>,
    clipboard: Option<&KeyBinding>,
) -> Result<TranslationShortcuts, String> {
    let binding = binding.map(checked_binding).transpose()?;
    if let Some(binding) = &binding {
        if clipboard.is_some_and(|clipboard| same_binding(binding, clipboard)) {
            return Err("shortcut_clipboard".into());
        }
        translation_conflict(shortcuts, Some(action), binding)?;
    }
    let mut next = shortcuts.clone();
    next.set(action, binding);
    Ok(next)
}

pub fn same_binding(left: &KeyBinding, right: &KeyBinding) -> bool {
    let Ok(left) = normalize_key(left.clone(), true) else {
        return false;
    };
    let Ok(right) = normalize_key(right.clone(), true) else {
        return false;
    };
    left.code == right.code && left.modifiers == right.modifiers
}

fn supported_code(code: &str) -> bool {
    if let Some(letter) = code.strip_prefix("Key") {
        return letter.len() == 1 && letter.bytes().all(|byte| byte.is_ascii_uppercase());
    }
    if let Some(digit) = code.strip_prefix("Digit") {
        return digit.len() == 1 && digit.bytes().all(|byte| byte.is_ascii_digit());
    }
    if let Some(function) = code.strip_prefix('F') {
        return function
            .parse::<u8>()
            .is_ok_and(|number| (1..=24).contains(&number) && !function.starts_with('0'));
    }
    matches!(
        code,
        "Enter"
            | "Space"
            | "Minus"
            | "Equal"
            | "BracketLeft"
            | "BracketRight"
            | "Backslash"
            | "Semicolon"
            | "Quote"
            | "Comma"
            | "Period"
            | "Slash"
            | "Backquote"
            | "ArrowUp"
            | "ArrowDown"
            | "ArrowLeft"
            | "ArrowRight"
    )
}

fn reserved(binding: &KeyBinding) -> bool {
    if matches!(binding.code.as_str(), "Tab" | "Escape") {
        return true;
    }
    let modifiers = binding.modifiers.join("+");
    let editing = matches!(
        binding.code.as_str(),
        "KeyA" | "KeyZ" | "KeyX" | "KeyC" | "KeyV" | "KeyQ" | "KeyW" | "Comma"
    );
    if editing && matches!(modifiers.as_str(), "meta" | "control") {
        return true;
    }
    matches!(modifiers.as_str(), "shift+meta" | "control+shift") && binding.code == "KeyZ"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn panel_shortcuts_preserve_disabled_bindings_and_reject_conflicts() {
        let mut shortcuts = ClipboardRecordShortcuts::default();
        let action = ClipboardRecordAction::Pin;
        let global = KeyBinding::new("KeyJ", &["meta"]);
        for conflict in [KeyBinding::new("KeyF", &["alt"]), global.clone()] {
            assert_eq!(
                shortcuts.set(action, Some(conflict), None, &[Some(&global)]),
                Err("shortcut_clipboard_panel".into())
            );
        }
        shortcuts.set(action, None, Some(false), &[]).unwrap();
        let new = KeyBinding::new("KeyG", &["alt"]);
        shortcuts.set(action, Some(new.clone()), None, &[]).unwrap();
        let stored = serde_json::to_string(&shortcuts).unwrap();
        let restored = ClipboardRecordShortcuts::from_store(Some(stored));
        assert_eq!(restored.pin.binding, Some(new));
        assert!(!restored.pin.enabled);
    }

    #[test]
    fn older_clipboard_preferences_keep_custom_bindings_and_gain_new_defaults() {
        let restored = ClipboardRecordShortcuts::from_store(Some(
            r#"{
            "favorite":{"binding":{"code":"KeyG","modifiers":["alt"]},"enabled":false},
            "pin":{"binding":null,"enabled":true}
        }"#
            .into(),
        ));
        assert_eq!(
            restored.favorite.binding,
            Some(KeyBinding::new("KeyG", &["alt"]))
        );
        assert!(!restored.favorite.enabled);
        assert!(restored.pin.binding.is_none());
        for (_, shortcut) in restored.entries().into_iter().filter(|(action, _)| {
            !matches!(
                action,
                ClipboardRecordAction::Favorite | ClipboardRecordAction::Pin
            )
        }) {
            let binding = shortcut.binding.clone().unwrap();
            assert_eq!(checked_record_binding(binding.clone()), Ok(binding));
            assert!(shortcut.enabled);
        }
    }

    #[test]
    fn ignores_retired_copy_and_paste_preferences_without_resetting_other_keys() {
        let restored = ClipboardRecordShortcuts::from_store(Some(
            r#"{
            "copy":{"binding":{"code":"KeyY","modifiers":["meta"]},"enabled":true},
            "paste":{"binding":{"code":"KeyJ","modifiers":["alt"]},"enabled":true},
            "pin":{"binding":{"code":"KeyJ","modifiers":["alt"]},"enabled":false}
        }"#
            .into(),
        ));
        assert_eq!(
            restored.pin.binding,
            Some(KeyBinding::new("KeyJ", &["alt"]))
        );
        assert!(!restored.pin.enabled);
        let saved = serde_json::to_value(restored).unwrap();
        assert!(saved.get("copy").is_none() && saved.get("paste").is_none());
        for action in ["copy", "paste"] {
            assert!(serde_json::from_value::<ClipboardRecordAction>(action.into()).is_err());
        }
    }

    #[test]
    fn adds_note_ocr_and_translate_without_overwriting_a_custom_key() {
        let mut stored = serde_json::to_value(ClipboardRecordShortcuts::default()).unwrap();
        let object = stored.as_object_mut().unwrap();
        let note = object.remove("note").unwrap();
        let ocr = object.remove("ocr").unwrap();
        let translate = object.remove("translate").unwrap();
        object.insert("pin".into(), note.clone());
        let restored = serde_json::to_value(ClipboardRecordShortcuts::from_store(Some(
            stored.to_string(),
        )))
        .unwrap();
        assert_eq!(restored["pin"], note);
        assert!(restored["note"]["binding"].is_null());
        assert_eq!(restored["ocr"], ocr);
        assert_eq!(restored["translate"], translate);
    }

    #[test]
    fn clipboard_shortcuts_reject_plain_keys_and_accept_modified_delete() {
        let mut shortcuts = ClipboardRecordShortcuts::default();
        for code in ["Space", "F6", "Enter", "ArrowDown", "KeyA", "Backspace"] {
            assert_eq!(
                shortcuts.set(
                    ClipboardRecordAction::Preview,
                    Some(KeyBinding::new(code, &[])),
                    None,
                    &[]
                ),
                Err("shortcut_modifier".into())
            );
        }
        let delete = KeyBinding::new("Backspace", &["alt"]);
        assert!(normalize_binding(delete.clone()).is_err());
        let copy = KeyBinding::new("KeyC", &["meta"]);
        assert_eq!(
            shortcuts.set(ClipboardRecordAction::Delete, Some(copy), None, &[]),
            Err("shortcut_reserved".into())
        );
        shortcuts
            .set(ClipboardRecordAction::Delete, Some(delete), None, &[])
            .unwrap();
    }

    #[test]
    fn upgrades_legacy_plain_keys_without_resetting_other_custom_preferences() {
        let mut legacy = ClipboardRecordShortcuts::default();
        let preview = legacy.preview.binding.clone();
        legacy.preview = RecordShortcut {
            binding: Some(KeyBinding::new("Space", &[])),
            enabled: false,
        };
        legacy.favorite.binding = Some(KeyBinding::new("KeyG", &["alt"]));
        legacy.delete.binding = Some(KeyBinding::new("F6", &[]));
        let reload = |legacy: &ClipboardRecordShortcuts| {
            ClipboardRecordShortcuts::from_store(Some(serde_json::to_string(legacy).unwrap()))
        };
        let restored = reload(&legacy);
        assert_eq!(restored.preview.binding, preview);
        assert!(!restored.preview.enabled);
        assert_eq!(restored.favorite.binding, legacy.favorite.binding);
        assert!(restored.delete.binding.is_none());
        legacy.favorite.binding = preview.clone();
        let restored = reload(&legacy);
        assert!(restored.preview.binding.is_none());
        assert_eq!(restored.favorite.binding, preview);
    }

    #[test]
    fn updates_legacy_option_delete_without_changing_its_enabled_state() {
        let mut legacy = ClipboardRecordShortcuts::default();
        let default_delete = legacy.delete.binding.clone();
        legacy.delete = RecordShortcut {
            binding: Some(KeyBinding::new("Backspace", &["alt"])),
            enabled: false,
        };
        let restored =
            ClipboardRecordShortcuts::from_store(Some(serde_json::to_string(&legacy).unwrap()));
        assert_eq!(restored.delete.binding, default_delete);
        assert!(!restored.delete.enabled);
        legacy.delete.binding = Some(KeyBinding::new("KeyJ", &["alt"]));
        let restored =
            ClipboardRecordShortcuts::from_store(Some(serde_json::to_string(&legacy).unwrap()));
        assert_eq!(restored.delete.binding, legacy.delete.binding);
    }

    #[test]
    fn defaults_follow_the_platform_modifier_and_keep_current_bindings() {
        let shortcuts = TranslationShortcuts::defaults("meta");
        assert_eq!(
            shortcuts.selection,
            Some(KeyBinding::new("KeyT", &["meta"]))
        );
        assert_eq!(
            shortcuts.translate,
            Some(KeyBinding::new("Enter", &["meta"]))
        );
        assert_eq!(
            shortcuts.pin,
            Some(KeyBinding::new("KeyP", &["meta", "shift"]))
        );
        assert_eq!(
            TranslationShortcuts::defaults("control").actions,
            Some(KeyBinding::new("KeyK", &["control"]))
        );
    }

    #[test]
    fn upgrades_shared_shortcuts_without_overwriting_existing_assignments() {
        let mut old = serde_json::to_value(TranslationShortcuts::defaults("meta")).unwrap();
        let custom_pin = KeyBinding::new("KeyJ", &["alt"]);
        old["pin"] = serde_json::to_value(&custom_pin).unwrap();
        old.as_object_mut().unwrap().remove("showShortcuts");
        old.as_object_mut().unwrap().remove("focusSearch");
        let load = |old: &serde_json::Value,
                    global: Option<KeyBinding>,
                    records: ClipboardRecordShortcuts| {
            TranslationShortcuts::from_store(
                |key| match key {
                    "translation_shortcuts" => Some(old.to_string()),
                    "clipboard_shortcut" => Some(serde_json::to_string(&global).unwrap()),
                    "clipboard_record_shortcuts" => Some(serde_json::to_string(&records).unwrap()),
                    _ => None,
                },
                "meta",
            )
        };
        let records = ClipboardRecordShortcuts::default();
        let restored = load(&old, None, records.clone());
        assert_eq!(restored.pin, Some(custom_pin));
        assert_eq!(
            restored.show_shortcuts,
            Some(KeyBinding::new("Slash", &["meta"]))
        );
        assert_eq!(
            restored.focus_search,
            Some(KeyBinding::new("KeyF", &["meta"]))
        );

        let mut records = records;
        records.preview.binding = restored.focus_search.clone();
        let restored = load(&old, restored.show_shortcuts, records);
        assert!(restored.show_shortcuts.is_none());
        assert!(restored.focus_search.is_none());

        old["focusSearch"] = serde_json::Value::Null;
        old["actions"] = serde_json::to_value(KeyBinding::new("Slash", &["meta"])).unwrap();
        let restored = load(&old, None, ClipboardRecordShortcuts::default());
        assert!(restored.focus_search.is_none());
        assert!(restored.show_shortcuts.is_none());
        assert_eq!(restored.actions, Some(KeyBinding::new("Slash", &["meta"])));
    }

    #[test]
    fn clipboard_and_translation_respect_each_others_current_bindings() {
        let translations = TranslationShortcuts::defaults("meta");
        let new_clipboard = KeyBinding::new("KeyJ", &["control"]);
        assert_eq!(
            validate_clipboard_binding(KeyBinding::new("KeyT", &["meta"]), &translations),
            Err("shortcut_conflict:selection".into())
        );
        assert_eq!(
            validate_clipboard_binding(new_clipboard.clone(), &translations),
            Ok(new_clipboard.clone())
        );
        assert_eq!(
            assign_shortcut(
                &translations,
                ShortcutAction::Selection,
                Some(new_clipboard.clone()),
                Some(&new_clipboard)
            ),
            Err("shortcut_clipboard".into())
        );
        assert!(assign_shortcut(
            &translations,
            ShortcutAction::Selection,
            Some(clipboard_binding()),
            Some(&new_clipboard)
        )
        .is_ok());
    }

    #[test]
    fn rejects_reserved_clipboard_and_duplicate_shortcuts() {
        let shortcuts = TranslationShortcuts::defaults("meta");
        assert_eq!(
            assign_shortcut(
                &shortcuts,
                ShortcutAction::Translate,
                Some(KeyBinding::new("KeyW", &["meta"])),
                Some(&clipboard_binding())
            )
            .unwrap_err(),
            "shortcut_reserved"
        );
        assert_eq!(
            assign_shortcut(
                &shortcuts,
                ShortcutAction::Translate,
                Some(KeyBinding::new("KeyZ", &["control", "shift"])),
                Some(&clipboard_binding())
            )
            .unwrap_err(),
            "shortcut_reserved"
        );
        assert_eq!(
            assign_shortcut(
                &shortcuts,
                ShortcutAction::Translate,
                Some(KeyBinding::new("KeyZ", &["meta", "shift"])),
                Some(&clipboard_binding())
            )
            .unwrap_err(),
            "shortcut_reserved"
        );
        assert_eq!(
            assign_shortcut(
                &shortcuts,
                ShortcutAction::Actions,
                Some(KeyBinding::new("Backquote", &["control"])),
                Some(&clipboard_binding())
            )
            .unwrap_err(),
            "shortcut_clipboard"
        );
        assert_eq!(
            assign_shortcut(
                &shortcuts,
                ShortcutAction::Pin,
                Some(KeyBinding::new("Enter", &["meta"])),
                Some(&clipboard_binding())
            )
            .unwrap_err(),
            "shortcut_conflict:translate"
        );
        assert_eq!(
            assign_shortcut(
                &shortcuts,
                ShortcutAction::Translate,
                Some(KeyBinding::new("KeyP", &["shift"])),
                Some(&clipboard_binding())
            )
            .unwrap_err(),
            "shortcut_modifier"
        );
        let cleared = assign_shortcut(
            &shortcuts,
            ShortcutAction::Actions,
            None,
            Some(&clipboard_binding()),
        )
        .unwrap();
        assert_eq!(cleared.actions, None);
        let rebound = assign_shortcut(
            &shortcuts,
            ShortcutAction::Selection,
            Some(KeyBinding::new("KeyJ", &["alt", "meta"])),
            Some(&clipboard_binding()),
        )
        .unwrap();
        assert_eq!(
            rebound.selection,
            Some(KeyBinding::new("KeyJ", &["alt", "meta"]))
        );
    }
}
