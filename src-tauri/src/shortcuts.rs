use serde::{Deserialize, Serialize};

pub const CLIPBOARD_SHORTCUT_CODE: &str = "Backquote";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ShortcutAction {
    Selection,
    OpenTranslation,
    Translate,
    CopyPrimary,
    CopySecondary,
    Pin,
    ShowShortcuts,
    FocusSearch,
    OpenSettings,
}

impl ShortcutAction {
    pub fn parse(value: &str) -> Result<Self, String> {
        Ok(match value {
            "selection" => Self::Selection,
            "openTranslation" => Self::OpenTranslation,
            "translate" => Self::Translate,
            "copyPrimary" => Self::CopyPrimary,
            "copySecondary" => Self::CopySecondary,
            "pin" => Self::Pin,
            "showShortcuts" => Self::ShowShortcuts,
            "focusSearch" => Self::FocusSearch,
            "openSettings" => Self::OpenSettings,
            _ => return Err("invalid_argument".into()),
        })
    }

    pub fn is_global_translation(self) -> bool {
        matches!(self, Self::Selection | Self::OpenTranslation)
    }

    pub fn is_global(self) -> bool {
        self.is_global_translation()
    }

    fn shared_with_clipboard(self) -> bool {
        self.is_global() || is_general_window_action(self)
    }

    fn overlaps_record_action(self, record: ClipboardRecordAction, binding: &KeyBinding) -> bool {
        if self.is_global_translation()
            && record == ClipboardRecordAction::Translate
            && is_clipboard_translate_launch_key(binding)
        {
            return false;
        }
        self.shared_with_clipboard() || record.shared_with_translation()
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Selection => "selection",
            Self::OpenTranslation => "openTranslation",
            Self::Translate => "translate",
            Self::CopyPrimary => "copyPrimary",
            Self::CopySecondary => "copySecondary",
            Self::Pin => "pin",
            Self::ShowShortcuts => "showShortcuts",
            Self::FocusSearch => "focusSearch",
            Self::OpenSettings => "openSettings",
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
    #[serde(default)]
    pub open_translation: Option<KeyBinding>,
    pub translate: Option<KeyBinding>,
    pub copy_primary: Option<KeyBinding>,
    pub copy_secondary: Option<KeyBinding>,
    pub pin: Option<KeyBinding>,
    #[serde(default)]
    pub show_shortcuts: Option<KeyBinding>,
    #[serde(default)]
    pub focus_search: Option<KeyBinding>,
    #[serde(default)]
    pub open_settings: Option<KeyBinding>,
}

impl TranslationShortcuts {
    pub fn defaults(primary: &str) -> Self {
        Self {
            selection: Some(KeyBinding::new("KeyT", &[primary])),
            open_translation: Some(KeyBinding::new("KeyT", &[primary])),
            translate: Some(KeyBinding::new("Enter", &[primary])),
            copy_primary: Some(KeyBinding::new("Digit1", &[primary])),
            copy_secondary: Some(KeyBinding::new("Digit2", &[primary])),
            pin: Some(KeyBinding::new("KeyP", &[primary, "shift"])),
            show_shortcuts: Some(KeyBinding::new("KeyK", &[primary])),
            focus_search: Some(KeyBinding::new("KeyF", &[primary])),
            open_settings: Some(KeyBinding::new("Comma", &[primary])),
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
        if keys.get("openTranslation").is_none() {
            shortcuts.open_translation = shortcuts.selection.clone();
        }
        for action in TRANSLATION_ACTIONS {
            if shortcuts.get(action).is_some_and(is_native_close_binding) {
                shortcuts.set(action, None);
            }
        }
        let clipboard = read("clipboard_shortcut")
            .and_then(|value| serde_json::from_str::<Option<KeyBinding>>(&value).ok())
            .unwrap_or_else(|| Some(clipboard_binding()));
        let records = ClipboardRecordShortcuts::from_store(read("clipboard_record_shortcuts"));
        for action in [
            ShortcutAction::ShowShortcuts,
            ShortcutAction::FocusSearch,
            ShortcutAction::OpenSettings,
        ] {
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

    pub fn bindings_for_record(
        &self,
        record: ClipboardRecordAction,
    ) -> impl Iterator<Item = &KeyBinding> {
        TRANSLATION_ACTIONS.into_iter().filter_map(move |action| {
            self.get(action)
                .filter(|binding| action.overlaps_record_action(record, binding))
        })
    }

    pub fn conflicts_with_records(&self, records: &ClipboardRecordShortcuts) -> bool {
        TRANSLATION_ACTIONS.into_iter().any(|action| {
            self.get(action)
                .is_some_and(|key| records.conflicts_with_translation(action, key))
        })
    }

    pub fn restore_group(
        &self,
        general: bool,
        primary: &str,
        clipboard: Option<&KeyBinding>,
    ) -> Result<Self, String> {
        let defaults = Self::defaults(primary);
        let actions = TRANSLATION_ACTIONS
            .into_iter()
            .filter(|action| is_general_window_action(*action) == general);
        let mut next = self.clone();
        for action in actions.clone() {
            next.set(action, None);
        }
        for action in actions {
            next = assign_shortcut(&next, action, defaults.get(action).cloned(), clipboard)?;
        }
        Ok(next)
    }

    pub fn get(&self, action: ShortcutAction) -> Option<&KeyBinding> {
        match action {
            ShortcutAction::Selection => self.selection.as_ref(),
            ShortcutAction::OpenTranslation => self.open_translation.as_ref(),
            ShortcutAction::Translate => self.translate.as_ref(),
            ShortcutAction::CopyPrimary => self.copy_primary.as_ref(),
            ShortcutAction::CopySecondary => self.copy_secondary.as_ref(),
            ShortcutAction::Pin => self.pin.as_ref(),
            ShortcutAction::ShowShortcuts => self.show_shortcuts.as_ref(),
            ShortcutAction::FocusSearch => self.focus_search.as_ref(),
            ShortcutAction::OpenSettings => self.open_settings.as_ref(),
        }
    }

    pub fn set(&mut self, action: ShortcutAction, binding: Option<KeyBinding>) {
        let slot = match action {
            ShortcutAction::Selection => &mut self.selection,
            ShortcutAction::OpenTranslation => &mut self.open_translation,
            ShortcutAction::Translate => &mut self.translate,
            ShortcutAction::CopyPrimary => &mut self.copy_primary,
            ShortcutAction::CopySecondary => &mut self.copy_secondary,
            ShortcutAction::Pin => &mut self.pin,
            ShortcutAction::ShowShortcuts => &mut self.show_shortcuts,
            ShortcutAction::FocusSearch => &mut self.focus_search,
            ShortcutAction::OpenSettings => &mut self.open_settings,
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
    OpenTarget,
    Translate,
    Favorite,
    Pin,
    Delete,
    CopyRecord,
    PastePlain,
    SaveImage,
    SwitchCategory,
}

impl ClipboardRecordAction {
    pub fn shared_with_translation(self) -> bool {
        !matches!(
            self,
            Self::Ocr
                | Self::OpenTarget
                | Self::Translate
                | Self::PastePlain
                | Self::SaveImage
                | Self::SwitchCategory
        )
    }

    fn requires_safe_menu_key(self) -> bool {
        matches!(self, Self::PastePlain | Self::SaveImage)
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct RecordShortcut {
    pub binding: Option<KeyBinding>,
    pub enabled: bool,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ClipboardRecordShortcuts {
    preview: RecordShortcut,
    note: RecordShortcut,
    ocr: RecordShortcut,
    open_target: RecordShortcut,
    translate: RecordShortcut,
    favorite: RecordShortcut,
    pin: RecordShortcut,
    delete: RecordShortcut,
    copy_record: RecordShortcut,
    paste_plain: RecordShortcut,
    save_image: RecordShortcut,
    switch_category: RecordShortcut,
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
            preview: enabled("Space", &[]),
            note: enabled("KeyN", &[primary]),
            ocr: enabled("KeyO", &shifted),
            open_target: enabled("KeyO", &[primary]),
            translate: enabled(
                "KeyT",
                if primary == "meta" {
                    &["meta"]
                } else {
                    &shifted
                },
            ),
            favorite: enabled("KeyD", &[primary]),
            pin: enabled("KeyP", &[primary]),
            delete: enabled("Backspace", &[primary]),
            copy_record: enabled("KeyC", &[primary]),
            paste_plain: enabled("Enter", &[primary]),
            save_image: enabled("KeyS", &[primary]),
            switch_category: enabled("Tab", &[]),
        }
    }
}

impl ClipboardRecordShortcuts {
    fn entries(&self) -> [(ClipboardRecordAction, &RecordShortcut); 12] {
        use ClipboardRecordAction::*;
        [
            (Preview, &self.preview),
            (Note, &self.note),
            (Ocr, &self.ocr),
            (OpenTarget, &self.open_target),
            (Translate, &self.translate),
            (Favorite, &self.favorite),
            (Pin, &self.pin),
            (Delete, &self.delete),
            (CopyRecord, &self.copy_record),
            (PastePlain, &self.paste_plain),
            (SaveImage, &self.save_image),
            (SwitchCategory, &self.switch_category),
        ]
    }

    pub fn uses_translate_launch_key(&self) -> bool {
        self.translate.enabled
            && self
                .translate
                .binding
                .as_ref()
                .is_some_and(is_clipboard_translate_launch_key)
    }

    pub fn upgrade_translate_binding(
        &mut self,
        translation: &TranslationShortcuts,
        clipboard: Option<&KeyBinding>,
    ) {
        if cfg!(target_os = "macos")
            && self
                .translate
                .binding
                .as_ref()
                .is_some_and(|key| same_binding(key, &KeyBinding::new("KeyT", &["shift", "meta"])))
        {
            let reserved = std::iter::once(clipboard)
                .chain(
                    translation
                        .bindings_for_record(ClipboardRecordAction::Translate)
                        .map(Some),
                )
                .collect::<Vec<_>>();
            let _ = self.set(
                ClipboardRecordAction::Translate,
                Some(KeyBinding::new("KeyT", &["meta"])),
                None,
                &reserved,
            );
        }
    }

    pub fn restore_clipboard(&self) -> Result<Self, String> {
        let defaults = Self::default();
        let restored = Self {
            ocr: defaults.ocr,
            open_target: defaults.open_target,
            translate: defaults.translate,
            paste_plain: defaults.paste_plain,
            save_image: defaults.save_image,
            switch_category: defaults.switch_category,
            ..self.clone()
        };
        restored.validate_unique()?;
        Ok(restored)
    }

    pub fn restore_general(&self) -> Result<Self, String> {
        let restored = Self {
            ocr: self.ocr.clone(),
            open_target: self.open_target.clone(),
            translate: self.translate.clone(),
            paste_plain: self.paste_plain.clone(),
            save_image: self.save_image.clone(),
            switch_category: self.switch_category.clone(),
            ..Self::default()
        };
        restored.validate_unique()?;
        Ok(restored)
    }

    fn validate_unique(&self) -> Result<(), String> {
        for (action, shortcut) in self.entries() {
            if shortcut
                .binding
                .as_ref()
                .is_some_and(|binding| self.conflicts(binding, Some(action)))
            {
                return Err("shortcut_clipboard_panel".into());
            }
        }
        Ok(())
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

    pub fn conflicts_with_translation(
        &self,
        translation: ShortcutAction,
        binding: &KeyBinding,
    ) -> bool {
        self.entries().into_iter().any(|(action, shortcut)| {
            translation.overlaps_record_action(action, binding)
                && shortcut
                    .binding
                    .as_ref()
                    .is_some_and(|key| same_binding(key, binding))
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
        let next = binding.map(normalize_binding).transpose()?;
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
            validate_window_reservation(value)?;
            validate_quick_paste_reservation(value)?;
            if action.requires_safe_menu_key()
                && (matches!(value.code.as_str(), "Escape" | "Tab")
                    || value.modifiers.is_empty()
                        && matches!(
                            value.code.as_str(),
                            "Space" | "ArrowUp" | "ArrowDown" | "Home" | "End" | "Enter"
                        )
                    || cfg!(target_os = "macos")
                        && value.code == "KeyV"
                        && (value.modifiers == ["meta"] || value.modifiers == ["shift", "meta"]))
            {
                return Err("shortcut_reserved_record_menu".into());
            }
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
            ClipboardRecordAction::OpenTarget => &mut self.open_target,
            ClipboardRecordAction::Translate => &mut self.translate,
            ClipboardRecordAction::Favorite => &mut self.favorite,
            ClipboardRecordAction::Pin => &mut self.pin,
            ClipboardRecordAction::Delete => &mut self.delete,
            ClipboardRecordAction::CopyRecord => &mut self.copy_record,
            ClipboardRecordAction::PastePlain => &mut self.paste_plain,
            ClipboardRecordAction::SaveImage => &mut self.save_image,
            ClipboardRecordAction::SwitchCategory => &mut self.switch_category,
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
                let reserved = shortcuts.entries().map(|(action, slot)| {
                    slot.binding
                        .as_ref()
                        .filter(|binding| is_native_close_binding(binding))
                        .map(|_| action)
                });
                for action in reserved.into_iter().flatten() {
                    let _ = shortcuts.set(action, None, None, &[]);
                }
                // Missing defaults must not displace any saved binding.
                for (key, action) in [
                    ("preview", ClipboardRecordAction::Preview),
                    ("note", ClipboardRecordAction::Note),
                    ("favorite", ClipboardRecordAction::Favorite),
                    ("pin", ClipboardRecordAction::Pin),
                    ("delete", ClipboardRecordAction::Delete),
                    ("ocr", ClipboardRecordAction::Ocr),
                    ("openTarget", ClipboardRecordAction::OpenTarget),
                    ("translate", ClipboardRecordAction::Translate),
                    ("copyRecord", ClipboardRecordAction::CopyRecord),
                    ("pastePlain", ClipboardRecordAction::PastePlain),
                    ("saveImage", ClipboardRecordAction::SaveImage),
                    ("switchCategory", ClipboardRecordAction::SwitchCategory),
                ] {
                    if value.get(key).is_none() {
                        let binding =
                            shortcuts
                                .entries()
                                .into_iter()
                                .find_map(|(candidate, slot)| {
                                    (candidate == action)
                                        .then_some(slot.binding.as_ref())
                                        .flatten()
                                });
                        if binding.is_some_and(|binding| shortcuts.conflicts(binding, Some(action)))
                        {
                            let _ = shortcuts.set(action, None, None, &[]);
                        }
                    }
                }
                Some(shortcuts)
            })
            .filter(|shortcuts| {
                shortcuts.entries().into_iter().all(|(action, current)| {
                    current.binding.as_ref().is_none_or(|binding| {
                        normalize_binding(binding.clone()).is_ok()
                            && !shortcuts.conflicts(binding, Some(action))
                    })
                })
            })
            .unwrap_or_default()
    }
}

pub fn clipboard_binding() -> KeyBinding {
    KeyBinding::new(CLIPBOARD_SHORTCUT_CODE, &["control"])
}

pub fn normalize_binding(binding: KeyBinding) -> Result<KeyBinding, String> {
    let code = if binding.code == "NumpadEnter" {
        "Enter".to_string()
    } else {
        binding.code
    };
    if !supported_code(&code) {
        return Err("shortcut_key".into());
    }
    let mut modifiers = Vec::new();
    for name in ["control", "alt", "shift", "meta"] {
        if binding.modifiers.iter().any(|modifier| modifier == name) {
            modifiers.push(name.to_string());
        }
    }
    if modifiers.len() != binding.modifiers.len() {
        return Err("shortcut_modifier".into());
    }
    Ok(KeyBinding { code, modifiers })
}

const TRANSLATION_ACTIONS: [ShortcutAction; 9] = [
    ShortcutAction::OpenTranslation,
    ShortcutAction::Selection,
    ShortcutAction::Translate,
    ShortcutAction::CopyPrimary,
    ShortcutAction::CopySecondary,
    ShortcutAction::Pin,
    ShortcutAction::ShowShortcuts,
    ShortcutAction::FocusSearch,
    ShortcutAction::OpenSettings,
];

pub fn is_general_window_action(action: ShortcutAction) -> bool {
    matches!(
        action,
        ShortcutAction::Pin
            | ShortcutAction::ShowShortcuts
            | ShortcutAction::FocusSearch
            | ShortcutAction::OpenSettings
    )
}

fn translation_conflict(
    shortcuts: &TranslationShortcuts,
    except: Option<ShortcutAction>,
    binding: &KeyBinding,
) -> Result<(), String> {
    for action in TRANSLATION_ACTIONS {
        if Some(action) != except
            && !(action.is_global_translation()
                && except.is_some_and(ShortcutAction::is_global_translation))
            && shortcuts
                .get(action)
                .is_some_and(|current| same_binding(current, binding))
        {
            return Err(format!("shortcut_conflict:{}", action.as_str()));
        }
    }
    Ok(())
}

fn is_native_close_binding(binding: &KeyBinding) -> bool {
    cfg!(target_os = "macos") && binding.code == "KeyW" && binding.modifiers == ["meta"]
}

fn validate_window_reservation(binding: &KeyBinding) -> Result<(), String> {
    if is_native_close_binding(binding) {
        return Err("shortcut_reserved_close_window".into());
    }
    Ok(())
}

pub fn validate_quick_paste_reservation(binding: &KeyBinding) -> Result<(), String> {
    if cfg!(target_os = "macos")
        && binding.modifiers == ["meta"]
        && matches!(
            binding.code.as_str(),
            "Digit1" | "Digit2" | "Digit3" | "Digit4" | "Digit5"
        )
    {
        return Err("shortcut_reserved_quick_paste".into());
    }
    Ok(())
}

pub fn validate_clipboard_binding(
    binding: KeyBinding,
    shortcuts: &TranslationShortcuts,
) -> Result<KeyBinding, String> {
    let binding = normalize_binding(binding)?;
    validate_window_reservation(&binding)?;
    translation_conflict(shortcuts, None, &binding)?;
    Ok(binding)
}

pub fn assign_shortcut(
    shortcuts: &TranslationShortcuts,
    action: ShortcutAction,
    binding: Option<KeyBinding>,
    clipboard: Option<&KeyBinding>,
) -> Result<TranslationShortcuts, String> {
    let binding = binding.map(normalize_binding).transpose()?;
    if let Some(binding) = &binding {
        validate_window_reservation(binding)?;
        if action.is_global() || is_general_window_action(action) {
            validate_quick_paste_reservation(binding)?;
        }
        if clipboard.is_some_and(|clipboard| same_binding(binding, clipboard)) {
            return Err("shortcut_clipboard".into());
        }
        translation_conflict(shortcuts, Some(action), binding)?;
    }
    let mut next = shortcuts.clone();
    next.set(action, binding);
    Ok(next)
}

fn is_clipboard_translate_launch_key(binding: &KeyBinding) -> bool {
    cfg!(target_os = "macos") && binding.code == "KeyT" && binding.modifiers == ["meta"]
}

pub fn same_binding(left: &KeyBinding, right: &KeyBinding) -> bool {
    let Ok(left) = normalize_binding(left.clone()) else {
        return false;
    };
    let Ok(right) = normalize_binding(right.clone()) else {
        return false;
    };
    left.code == right.code && left.modifiers == right.modifiers
}

fn supported_code(code: &str) -> bool {
    use tauri_plugin_global_shortcut::Code;
    code.parse::<Code>()
        .is_ok_and(|key| key != Code::Unidentified && key.to_string() == code)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn image_and_plain_paste_shortcuts_persist_and_yield_to_existing_bindings() {
        use ClipboardRecordAction::*;
        let defaults = ClipboardRecordShortcuts::default();
        let mut legacy = serde_json::to_value(&defaults).unwrap();
        let legacy = legacy.as_object_mut().unwrap();
        legacy.remove("saveImage");
        legacy.insert(
            "pin".into(),
            serde_json::to_value(&defaults.save_image).unwrap(),
        );
        let loaded =
            ClipboardRecordShortcuts::from_store(Some(serde_json::to_string(legacy).unwrap()));
        assert_eq!(loaded.pin.binding, defaults.save_image.binding);
        assert_eq!(loaded.save_image.binding, None);

        let mut custom = ClipboardRecordShortcuts::default();
        let key = KeyBinding::new("KeyJ", &["alt"]);
        custom
            .set(PastePlain, Some(key.clone()), None, &[])
            .unwrap();
        custom.set(SaveImage, None, None, &[]).unwrap();
        custom.set(PastePlain, None, Some(false), &[]).unwrap();
        assert!(custom.set(SaveImage, Some(key.clone()), None, &[]).is_err());
        assert!(custom
            .set(SaveImage, Some(KeyBinding::new("Escape", &[])), None, &[])
            .is_err());
        assert!(custom
            .set(PastePlain, Some(KeyBinding::new("Enter", &[])), None, &[])
            .is_err());
        let reopened =
            ClipboardRecordShortcuts::from_store(Some(serde_json::to_string(&custom).unwrap()));
        assert_eq!(reopened.paste_plain.binding, Some(key));
        assert!(!reopened.paste_plain.enabled);
        assert!(reopened.save_image.binding.is_none());
        assert_eq!(reopened.restore_general().unwrap().save_image.binding, None);
        assert_eq!(
            reopened.restore_clipboard().unwrap().save_image.binding,
            defaults.save_image.binding
        );
    }

    #[test]
    fn record_conflicts_follow_their_context_in_both_directions() {
        use ClipboardRecordAction::{PastePlain, Preview};
        use ShortcutAction::{CopyPrimary, CopySecondary, FocusSearch, OpenTranslation, Translate};
        let mut translation = TranslationShortcuts::defaults("meta");
        let mut records = ClipboardRecordShortcuts::default();
        let key = KeyBinding::new("KeyJ", &["alt"]);
        records
            .set(PastePlain, Some(key.clone()), None, &[])
            .unwrap();
        for action in [
            Translate,
            CopyPrimary,
            CopySecondary,
            FocusSearch,
            OpenTranslation,
        ] {
            translation.set(action, Some(key.clone()));
            let common = matches!(action, FocusSearch | OpenTranslation);
            assert_eq!(
                translation
                    .bindings_for_record(PastePlain)
                    .any(|used| used == &key),
                common
            );
            assert_eq!(records.conflicts_with_translation(action, &key), common);
            assert_eq!(translation.conflicts_with_records(&records), common);
            assert!(translation
                .bindings_for_record(Preview)
                .any(|used| used == &key));
            translation = TranslationShortcuts::defaults("meta");
        }
        records.set(PastePlain, None, None, &[]).unwrap();
        records.set(Preview, Some(key.clone()), None, &[]).unwrap();
        for action in [Translate, CopyPrimary, CopySecondary] {
            translation.set(action, Some(key.clone()));
            assert!(records.conflicts_with_translation(action, &key));
            assert!(translation.conflicts_with_records(&records));
        }
    }

    #[test]
    fn switch_category_defaults_to_tab_unless_that_binding_is_taken() {
        use ClipboardRecordAction::SwitchCategory;
        let tab = KeyBinding::new("Tab", &[]);
        assert_eq!(
            ClipboardRecordShortcuts::default().switch_category.binding,
            Some(tab.clone())
        );
        let mut stored = serde_json::to_value(ClipboardRecordShortcuts::default()).unwrap();
        stored.as_object_mut().unwrap().remove("switchCategory");
        let loaded = ClipboardRecordShortcuts::from_store(Some(stored.to_string()));
        assert_eq!(loaded.switch_category.binding, Some(tab.clone()));
        let custom = KeyBinding::new("KeyJ", &[]);
        let mut changed = loaded;
        changed
            .set(SwitchCategory, Some(custom.clone()), None, &[])
            .unwrap();
        assert_eq!(
            changed.restore_general().unwrap().switch_category.binding,
            Some(custom)
        );
        assert_eq!(
            changed.restore_clipboard().unwrap().switch_category.binding,
            Some(tab.clone())
        );
        stored["preview"]["binding"] = serde_json::to_value(&tab).unwrap();
        let yielded = ClipboardRecordShortcuts::from_store(Some(stored.to_string()));
        assert_eq!(yielded.preview.binding, Some(tab));
        assert!(yielded.switch_category.binding.is_none());
    }

    #[test]
    fn paste_plain_defaults_upgrade_without_overwriting_existing_assignments() {
        use ClipboardRecordAction::*;
        let primary = if cfg!(target_os = "macos") {
            "meta"
        } else {
            "control"
        };
        let plain = KeyBinding::new("Enter", &[primary]);
        let defaults = ClipboardRecordShortcuts::default();
        assert_eq!(defaults.paste_plain.binding.as_ref(), Some(&plain));
        assert!(!defaults.conflicts_with_translation(ShortcutAction::Translate, &plain));

        let mut legacy = serde_json::to_value(&defaults).unwrap();
        legacy.as_object_mut().unwrap().remove("pastePlain");
        legacy["pasteRecord"] =
            serde_json::json!({"binding":{"code":"Enter","modifiers":[]},"enabled":false});
        let load = |value: &serde_json::Value| {
            ClipboardRecordShortcuts::from_store(Some(value.to_string()))
        };
        let migrated = load(&legacy);
        assert_eq!(migrated.paste_plain.binding.as_ref(), Some(&plain));
        assert!(serde_json::to_value(&migrated)
            .unwrap()
            .get("pasteRecord")
            .is_none());

        legacy["pin"]["binding"] = serde_json::to_value(&plain).unwrap();
        let migrated = load(&legacy);
        assert_eq!(migrated.pin.binding, Some(plain.clone()));
        assert!(migrated.paste_plain.binding.is_none());

        let mut custom = defaults;
        let key = KeyBinding::new("KeyJ", &["alt"]);
        custom
            .set(PastePlain, Some(key.clone()), None, &[])
            .unwrap();
        let reopened = load(&serde_json::to_value(custom).unwrap());
        assert_eq!(
            reopened.restore_general().unwrap().paste_plain.binding,
            Some(key)
        );
        assert_eq!(
            reopened.restore_clipboard().unwrap().paste_plain.binding,
            Some(plain)
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn command_t_only_shares_clipboard_translate_with_translation_launch() {
        use ClipboardRecordAction::{Favorite, Translate};
        use ShortcutAction::{FocusSearch, OpenTranslation, Selection};
        let key = KeyBinding::new("KeyT", &["meta"]);
        let launch = TranslationShortcuts::defaults("meta");
        let mut records = ClipboardRecordShortcuts::default();
        assert!(records.uses_translate_launch_key());
        assert!(!launch.conflicts_with_records(&records));
        for action in [Selection, OpenTranslation] {
            assert!(!records.conflicts_with_translation(action, &key));
        }
        assert!(records.conflicts_with_translation(FocusSearch, &key));
        assert!(launch
            .bindings_for_record(Favorite)
            .any(|binding| binding == &key));
        assert!(!launch
            .bindings_for_record(Translate)
            .any(|binding| binding == &key));
        let custom = KeyBinding::new("KeyJ", &["meta"]);
        records
            .set(Translate, Some(custom.clone()), None, &[])
            .unwrap();
        records.upgrade_translate_binding(&launch, None);
        assert_eq!(records.translate.binding, Some(custom.clone()));
        assert!(records.conflicts_with_translation(Selection, &custom));
        let old = KeyBinding::new("KeyT", &["shift", "meta"]);
        records
            .set(Translate, Some(old.clone()), None, &[])
            .unwrap();
        records.set(Translate, None, Some(false), &[]).unwrap();
        records.upgrade_translate_binding(&launch, Some(&key));
        assert_eq!(records.translate.binding, Some(old));
        records.upgrade_translate_binding(&launch, None);
        assert_eq!(records.translate.binding, Some(key));
        assert!(!records.uses_translate_launch_key());
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn fixed_clipboard_digits_reserve_the_panel_without_replacing_translation_copies() {
        let shortcuts = TranslationShortcuts::defaults("meta");
        let digit = KeyBinding::new("Digit4", &["meta"]);
        assert_eq!(
            assign_shortcut(
                &shortcuts,
                ShortcutAction::OpenTranslation,
                Some(digit.clone()),
                None
            ),
            Err("shortcut_reserved_quick_paste".into())
        );
        assert!(
            assign_shortcut(&shortcuts, ShortcutAction::CopyPrimary, Some(digit), None).is_ok()
        );
        assert!(
            validate_quick_paste_reservation(&KeyBinding::new("Digit5", &["meta", "shift"]))
                .is_ok()
        );
    }

    #[test]
    fn panel_shortcuts_preserve_disabled_bindings_and_reject_conflicts() {
        let mut shortcuts = ClipboardRecordShortcuts::default();
        let action = ClipboardRecordAction::Pin;
        let global = KeyBinding::new("KeyJ", &["meta"]);
        for conflict in [shortcuts.favorite.binding.clone().unwrap(), global.clone()] {
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
            assert_eq!(normalize_binding(binding.clone()), Ok(binding));
            assert!(shortcut.enabled);
        }
    }

    #[test]
    fn shared_record_defaults_match_general_and_do_not_replace_saved_bindings() {
        let defaults = ClipboardRecordShortcuts::default();
        let primary = if cfg!(target_os = "macos") {
            "meta"
        } else {
            "control"
        };
        assert_eq!(
            defaults.preview.binding,
            Some(KeyBinding::new("Space", &[]))
        );
        for (slot, code) in [
            (&defaults.note, "KeyN"),
            (&defaults.copy_record, "KeyC"),
            (&defaults.favorite, "KeyD"),
            (&defaults.pin, "KeyP"),
        ] {
            assert_eq!(slot.binding, Some(KeyBinding::new(code, &[primary])));
            assert!(slot.enabled);
        }
        let old = serde_json::json!({
            "preview": {"binding": {"code": "KeyY", "modifiers": [primary]}, "enabled": false},
            "pin": {"binding": {"code": "KeyD", "modifiers": [primary]}, "enabled": true},
            "note": {"binding": null, "enabled": true},
        });
        let restored = ClipboardRecordShortcuts::from_store(Some(old.to_string()));
        let saved = serde_json::to_value(restored).unwrap();
        for key in ["preview", "pin", "note"] {
            assert_eq!(saved[key], old[key]);
        }
        assert!(saved["favorite"]["binding"].is_null());
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
    fn ignores_retired_navigation_and_paste_record_without_resetting_other_keys() {
        let mut old = serde_json::to_value(ClipboardRecordShortcuts::default()).unwrap();
        for retired in [
            "focusRecords",
            "pasteRecord",
            "navigateUp",
            "navigateDown",
            "previousCategory",
            "nextCategory",
        ] {
            old[retired] = serde_json::json!({
                "binding":{"code":"F8","modifiers":[]},"enabled":true
            });
            assert!(serde_json::from_value::<ClipboardRecordAction>(retired.into()).is_err());
        }
        old["preview"] =
            serde_json::json!({"binding":{"code":"KeyJ","modifiers":[]},"enabled":false});
        let restored = ClipboardRecordShortcuts::from_store(Some(old.to_string()));
        assert_eq!(restored.preview.binding, Some(KeyBinding::new("KeyJ", &[])));
        assert!(!restored.preview.enabled);
        let saved = serde_json::to_value(&restored).unwrap();
        for retired in [
            "focusRecords",
            "pasteRecord",
            "navigateUp",
            "navigateDown",
            "previousCategory",
            "nextCategory",
        ] {
            assert!(saved.get(retired).is_none());
        }
    }

    #[test]
    fn shared_shortcuts_preserve_existing_bindings_and_ignore_removed_switch_view() {
        let mut old = serde_json::to_value(ClipboardRecordShortcuts::default()).unwrap();
        old.as_object_mut().unwrap().remove("copyRecord");
        old["switchView"] =
            serde_json::json!({"binding":{"code":"Tab","modifiers":[]},"enabled":true});
        old["note"] =
            serde_json::to_value(ClipboardRecordShortcuts::default().copy_record).unwrap();
        let restored = ClipboardRecordShortcuts::from_store(Some(old.to_string()));
        assert!(restored.copy_record.binding.is_none());
        assert_eq!(serde_json::to_value(&restored.note).unwrap(), old["note"]);
        let reset = restored.restore_clipboard().unwrap();
        let saved = serde_json::to_value(&reset).unwrap();
        assert!(saved.get("switchView").is_none());
        assert!(saved["copyRecord"]["binding"].is_null());
        assert_eq!(reset.note.binding, restored.note.binding);
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
    fn open_target_default_yields_to_a_preexisting_binding() {
        let mut stored = serde_json::to_value(ClipboardRecordShortcuts::default()).unwrap();
        let open = stored
            .as_object_mut()
            .unwrap()
            .remove("openTarget")
            .unwrap();
        let restore = |stored: &serde_json::Value| {
            serde_json::to_value(ClipboardRecordShortcuts::from_store(Some(
                stored.to_string(),
            )))
            .unwrap()
        };
        assert_eq!(restore(&stored)["openTarget"], open);
        stored["copyRecord"] = open.clone();
        let restored = restore(&stored);
        assert_eq!(restored["copyRecord"], open);
        assert!(restored["openTarget"]["binding"].is_null());
    }

    #[test]
    fn plain_and_shift_only_shortcuts_survive_reload_and_navigation_reset_is_scoped() {
        use ClipboardRecordAction::*;
        let mut shortcuts = ClipboardRecordShortcuts::default();
        for (action, code, modifiers) in [
            (Preview, "Enter", &[][..]),
            (Delete, "Backspace", &["alt"][..]),
            (PastePlain, "KeyK", &["alt"][..]),
        ] {
            shortcuts
                .set(action, Some(KeyBinding::new(code, modifiers)), None, &[])
                .unwrap();
        }
        shortcuts.set(Preview, None, Some(false), &[]).unwrap();
        let saved = serde_json::to_value(&shortcuts).unwrap();
        let restored = ClipboardRecordShortcuts::from_store(Some(saved.to_string()));
        assert_eq!(serde_json::to_value(&restored).unwrap(), saved);
        let reset = restored.restore_general().unwrap();
        assert_eq!(
            reset.preview.binding,
            ClipboardRecordShortcuts::default().preview.binding
        );
        assert!(reset.preview.enabled);
        assert_eq!(
            reset.paste_plain.binding,
            Some(KeyBinding::new("KeyK", &["alt"]))
        );
        let mut restored = restored;
        let preview = KeyBinding::new("KeyJ", &[]);
        restored
            .set(Preview, Some(preview.clone()), None, &[])
            .unwrap();
        assert_eq!(
            restored.restore_clipboard().unwrap().preview.binding,
            Some(preview)
        );
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
            TranslationShortcuts::defaults("control").show_shortcuts,
            Some(KeyBinding::new("KeyK", &["control"]))
        );
    }

    #[test]
    fn settings_shortcut_upgrade_preserves_existing_comma_assignment() {
        let mut old = serde_json::to_value(TranslationShortcuts::defaults("meta")).unwrap();
        let comma = old.as_object_mut().unwrap().remove("openSettings").unwrap();
        let load = |old: &serde_json::Value| {
            TranslationShortcuts::from_store(
                |key| (key == "translation_shortcuts").then(|| old.to_string()),
                "meta",
            )
        };
        assert_eq!(
            load(&old).open_settings,
            Some(KeyBinding::new("Comma", &["meta"]))
        );
        old["focusSearch"] = comma;
        let settings = load(&old);
        assert!(settings.open_settings.is_none());
        assert_eq!(
            settings.focus_search,
            Some(KeyBinding::new("Comma", &["meta"]))
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
            Some(KeyBinding::new("KeyK", &["meta"]))
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
        old["toggleHistory"] = serde_json::to_value(KeyBinding::new("Slash", &["meta"])).unwrap();
        let restored = load(&old, None, ClipboardRecordShortcuts::default());
        assert!(restored.focus_search.is_none());
        assert_eq!(
            restored.show_shortcuts,
            Some(KeyBinding::new("KeyK", &["meta"]))
        );
    }

    #[test]
    fn open_and_autofill_can_share_a_key_but_cannot_share_other_actions() {
        use ShortcutAction::{OpenTranslation, Selection, Translate};
        let defaults = TranslationShortcuts::defaults("meta");
        let custom = Some(KeyBinding::new("F7", &[]));
        let next = assign_shortcut(&defaults, Selection, custom.clone(), None).unwrap();
        assert_eq!(next.get(OpenTranslation), defaults.get(OpenTranslation));
        let next = assign_shortcut(&next, OpenTranslation, custom.clone(), None).unwrap();
        assert_eq!(next.get(Selection), next.get(OpenTranslation));
        assert_eq!(
            assign_shortcut(&next, Translate, custom, None),
            Err("shortcut_conflict:openTranslation".into())
        );
        assert_eq!(next.restore_group(false, "meta", None).unwrap(), defaults);
    }

    #[test]
    fn clipboard_and_translation_respect_each_others_current_bindings() {
        let translations = TranslationShortcuts::defaults("meta");
        let new_clipboard = KeyBinding::new("KeyJ", &["control"]);
        assert_eq!(
            validate_clipboard_binding(KeyBinding::new("KeyT", &["meta"]), &translations),
            Err("shortcut_conflict:openTranslation".into())
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

    #[cfg(target_os = "macos")]
    #[test]
    fn reserves_command_w_for_native_close_across_shortcut_domains() {
        let translation = TranslationShortcuts::defaults("meta");
        let mut records = ClipboardRecordShortcuts::default();
        let close = KeyBinding::new("KeyW", &["meta"]);
        let before = serde_json::to_value(&records).unwrap();
        assert_eq!(
            assign_shortcut(
                &translation,
                ShortcutAction::Selection,
                Some(close.clone()),
                None
            ),
            Err("shortcut_reserved_close_window".into())
        );
        assert_eq!(
            validate_clipboard_binding(close.clone(), &translation),
            Err("shortcut_reserved_close_window".into())
        );
        assert_eq!(
            records.set(ClipboardRecordAction::Preview, Some(close), None, &[]),
            Err("shortcut_reserved_close_window".into())
        );
        assert_eq!(serde_json::to_value(records).unwrap(), before);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn clears_only_reserved_bindings_when_loading_existing_shortcuts() {
        let close = KeyBinding::new("KeyW", &["meta"]);
        let mut translation = TranslationShortcuts::defaults("meta");
        translation.selection = Some(close.clone());
        translation.translate = Some(close.clone());
        translation.pin = Some(KeyBinding::new("KeyW", &["meta", "shift"]));
        let mut stored = serde_json::to_value(&translation).unwrap();
        stored.as_object_mut().unwrap().remove("openTranslation");
        let restored = TranslationShortcuts::from_store(
            |key| (key == "translation_shortcuts").then(|| stored.to_string()),
            "meta",
        );
        translation.selection = None;
        translation.open_translation = None;
        translation.translate = None;
        assert_eq!(restored, translation);

        let mut records = ClipboardRecordShortcuts::default();
        records.preview.binding = Some(close);
        records.preview.enabled = false;
        records.note.binding = Some(KeyBinding::new("KeyJ", &["alt"]));
        let restored =
            ClipboardRecordShortcuts::from_store(Some(serde_json::to_string(&records).unwrap()));
        records.preview.binding = None;
        assert_eq!(
            serde_json::to_value(restored).unwrap(),
            serde_json::to_value(records).unwrap()
        );
    }

    #[test]
    fn rejects_duplicate_shortcuts_but_allows_local_editing_combinations() {
        let shortcuts = TranslationShortcuts::defaults("meta");
        for (code, modifiers) in [
            ("KeyW", &["control"][..]),
            ("KeyW", &["meta", "shift"][..]),
            ("KeyZ", &["control", "shift"][..]),
            ("KeyZ", &["meta", "shift"][..]),
        ] {
            let binding = KeyBinding::new(code, modifiers);
            assert!(assign_shortcut(
                &shortcuts,
                ShortcutAction::Translate,
                Some(binding),
                Some(&clipboard_binding())
            )
            .is_ok());
        }
        assert_eq!(
            assign_shortcut(
                &shortcuts,
                ShortcutAction::ShowShortcuts,
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
        let shift_only = KeyBinding::new("KeyP", &["shift"]);
        assert_eq!(
            assign_shortcut(
                &shortcuts,
                ShortcutAction::Translate,
                Some(shift_only.clone()),
                Some(&clipboard_binding())
            )
            .unwrap()
            .translate,
            Some(shift_only)
        );
        let cleared = assign_shortcut(
            &shortcuts,
            ShortcutAction::ShowShortcuts,
            None,
            Some(&clipboard_binding()),
        )
        .unwrap();
        assert_eq!(cleared.show_shortcuts, None);
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
