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
    ToggleHistory,
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
            "toggleHistory" => Self::ToggleHistory,
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
        self.is_global_translation() || self == Self::ToggleHistory
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
            Self::ToggleHistory => "toggleHistory",
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
    pub toggle_history: Option<KeyBinding>,
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
            toggle_history: Some(KeyBinding::new("Slash", &[primary])),
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
        // Move the old defaults together, leaving custom or cleared bindings alone.
        if shortcuts.show_shortcuts == defaults.toggle_history {
            if shortcuts.toggle_history == defaults.show_shortcuts {
                std::mem::swap(&mut shortcuts.toggle_history, &mut shortcuts.show_shortcuts);
            } else if keys.get("toggleHistory").is_none() {
                let binding = defaults.show_shortcuts.clone();
                if !binding
                    .as_ref()
                    .is_some_and(|key| records.conflicts(key, None))
                {
                    if let Ok(next) = assign_shortcut(
                        &shortcuts,
                        ShortcutAction::ShowShortcuts,
                        binding,
                        clipboard.as_ref(),
                    ) {
                        shortcuts = next;
                    }
                }
            }
        }
        for action in [
            ShortcutAction::ShowShortcuts,
            ShortcutAction::FocusSearch,
            ShortcutAction::OpenSettings,
            ShortcutAction::ToggleHistory,
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
            ShortcutAction::ToggleHistory => self.toggle_history.as_ref(),
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
            ShortcutAction::ToggleHistory => &mut self.toggle_history,
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
    PasteRecord,
    PastePlain,
    SaveImage,
    SwitchView,
    NavigateUp,
    NavigateDown,
    PreviousCategory,
    NextCategory,
}

impl ClipboardRecordAction {
    pub fn shared_with_translation(self) -> bool {
        !matches!(self, Self::Ocr | Self::OpenTarget | Self::Translate) && !self.menu_only()
    }

    fn menu_only(self) -> bool {
        matches!(self, Self::PasteRecord | Self::PastePlain | Self::SaveImage)
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
    paste_record: RecordShortcut,
    paste_plain: RecordShortcut,
    save_image: RecordShortcut,
    switch_view: RecordShortcut,
    navigate_up: RecordShortcut,
    navigate_down: RecordShortcut,
    previous_category: RecordShortcut,
    next_category: RecordShortcut,
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
            open_target: enabled("KeyO", &[primary]),
            translate: enabled(
                "KeyT",
                if primary == "meta" {
                    &["meta"]
                } else {
                    &shifted
                },
            ),
            favorite: enabled("KeyF", &["alt"]),
            pin: enabled("KeyP", &["alt"]),
            delete: enabled("Backspace", &[primary]),
            copy_record: enabled("KeyC", &shifted),
            paste_record: enabled("Enter", &[]),
            paste_plain: enabled("Enter", &[primary]),
            save_image: enabled("KeyS", &[primary]),
            switch_view: enabled("Tab", &[]),
            navigate_up: enabled("ArrowUp", &[]),
            navigate_down: enabled("ArrowDown", &[]),
            previous_category: enabled("ArrowLeft", &[]),
            next_category: enabled("ArrowRight", &[]),
        }
    }
}

impl ClipboardRecordShortcuts {
    fn entries(&self) -> [(ClipboardRecordAction, &RecordShortcut); 17] {
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
            (PasteRecord, &self.paste_record),
            (PastePlain, &self.paste_plain),
            (SaveImage, &self.save_image),
            (SwitchView, &self.switch_view),
            (NavigateUp, &self.navigate_up),
            (NavigateDown, &self.navigate_down),
            (PreviousCategory, &self.previous_category),
            (NextCategory, &self.next_category),
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
            paste_record: defaults.paste_record,
            paste_plain: defaults.paste_plain,
            save_image: defaults.save_image,
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
            paste_record: self.paste_record.clone(),
            paste_plain: self.paste_plain.clone(),
            save_image: self.save_image.clone(),
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
            if action.menu_only()
                && (matches!(value.code.as_str(), "Escape" | "Tab")
                    || value.modifiers.is_empty()
                        && (matches!(
                            value.code.as_str(),
                            "Space" | "ArrowUp" | "ArrowDown" | "Home" | "End"
                        ) || value.code == "Enter"
                            && action != ClipboardRecordAction::PasteRecord)
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
            ClipboardRecordAction::PasteRecord => &mut self.paste_record,
            ClipboardRecordAction::PastePlain => &mut self.paste_plain,
            ClipboardRecordAction::SaveImage => &mut self.save_image,
            ClipboardRecordAction::SwitchView => &mut self.switch_view,
            ClipboardRecordAction::NavigateUp => &mut self.navigate_up,
            ClipboardRecordAction::NavigateDown => &mut self.navigate_down,
            ClipboardRecordAction::PreviousCategory => &mut self.previous_category,
            ClipboardRecordAction::NextCategory => &mut self.next_category,
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
                // Newly added defaults yield to existing assignments on upgrade.
                for (key, slot) in [("note", &mut shortcuts.note), ("ocr", &mut shortcuts.ocr)] {
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
                for (key, action) in [
                    ("openTarget", ClipboardRecordAction::OpenTarget),
                    ("translate", ClipboardRecordAction::Translate),
                    ("copyRecord", ClipboardRecordAction::CopyRecord),
                    ("switchView", ClipboardRecordAction::SwitchView),
                    ("navigateUp", ClipboardRecordAction::NavigateUp),
                    ("navigateDown", ClipboardRecordAction::NavigateDown),
                    ("previousCategory", ClipboardRecordAction::PreviousCategory),
                    ("nextCategory", ClipboardRecordAction::NextCategory),
                    ("pasteRecord", ClipboardRecordAction::PasteRecord),
                    ("pastePlain", ClipboardRecordAction::PastePlain),
                    ("saveImage", ClipboardRecordAction::SaveImage),
                ] {
                    if value.get(key).is_none() {
                        let binding = shortcuts
                            .entries()
                            .into_iter()
                            .find(|(candidate, _)| *candidate == action)
                            .and_then(|(_, slot)| slot.binding.clone());
                        if binding
                            .as_ref()
                            .is_some_and(|binding| shortcuts.conflicts(binding, Some(action)))
                        {
                            let _ = shortcuts.set(action, None, None, &[]);
                        }
                    }
                }
                if value.get("pastePlain").is_none() {
                    let primary = if cfg!(target_os = "macos") {
                        "meta"
                    } else {
                        "control"
                    };
                    let previous = KeyBinding::new("Enter", &[primary, "shift"]);
                    if shortcuts
                        .paste_record
                        .binding
                        .as_ref()
                        .is_some_and(|binding| same_binding(binding, &previous))
                    {
                        // Keep the old binding if Return was assigned to another action.
                        let _ = shortcuts.set(
                            ClipboardRecordAction::PasteRecord,
                            Some(KeyBinding::new("Enter", &[])),
                            None,
                            &[],
                        );
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

const TRANSLATION_ACTIONS: [ShortcutAction; 10] = [
    ShortcutAction::OpenTranslation,
    ShortcutAction::Selection,
    ShortcutAction::Translate,
    ShortcutAction::CopyPrimary,
    ShortcutAction::CopySecondary,
    ShortcutAction::Pin,
    ShortcutAction::ToggleHistory,
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
    fn image_menu_shortcuts_persist_and_new_defaults_yield_to_existing_bindings() {
        use ClipboardRecordAction::*;
        let defaults = ClipboardRecordShortcuts::default();
        let mut legacy = serde_json::to_value(&defaults).unwrap();
        let legacy = legacy.as_object_mut().unwrap();
        legacy.remove("pasteRecord");
        legacy.remove("saveImage");
        legacy.insert(
            "pin".into(),
            serde_json::to_value(&defaults.save_image).unwrap(),
        );
        let loaded =
            ClipboardRecordShortcuts::from_store(Some(serde_json::to_string(legacy).unwrap()));
        assert_eq!(loaded.pin.binding, defaults.save_image.binding);
        assert_eq!(loaded.save_image.binding, None);
        assert_eq!(loaded.paste_record.binding, defaults.paste_record.binding);

        let mut custom = ClipboardRecordShortcuts::default();
        let key = KeyBinding::new("KeyJ", &["alt"]);
        custom
            .set(PasteRecord, Some(key.clone()), None, &[])
            .unwrap();
        custom.set(SaveImage, None, None, &[]).unwrap();
        custom.set(PasteRecord, None, Some(false), &[]).unwrap();
        assert!(custom.set(SaveImage, Some(key.clone()), None, &[]).is_err());
        assert!(custom
            .set(SaveImage, Some(KeyBinding::new("Escape", &[])), None, &[])
            .is_err());
        let reopened =
            ClipboardRecordShortcuts::from_store(Some(serde_json::to_string(&custom).unwrap()));
        assert_eq!(reopened.paste_record.binding, Some(key));
        assert!(!reopened.paste_record.enabled);
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
    fn paste_menu_defaults_upgrade_without_overwriting_existing_assignments() {
        use ClipboardRecordAction::*;
        let primary = if cfg!(target_os = "macos") {
            "meta"
        } else {
            "control"
        };
        let old_key = KeyBinding::new("Enter", &[primary, "shift"]);
        let paste = KeyBinding::new("Enter", &[]);
        let plain = KeyBinding::new("Enter", &[primary]);
        let defaults = ClipboardRecordShortcuts::default();
        assert_eq!(defaults.paste_record.binding.as_ref(), Some(&paste));
        assert_eq!(defaults.paste_plain.binding.as_ref(), Some(&plain));
        assert!(!defaults.conflicts_with_translation(ShortcutAction::Translate, &plain));

        let mut legacy = serde_json::to_value(&defaults).unwrap();
        legacy.as_object_mut().unwrap().remove("pastePlain");
        legacy["pasteRecord"] = serde_json::json!({"binding":old_key,"enabled":false});
        let load = |value: &serde_json::Value| {
            ClipboardRecordShortcuts::from_store(Some(value.to_string()))
        };
        let migrated = load(&legacy);
        assert_eq!(migrated.paste_record.binding.as_ref(), Some(&paste));
        assert!(!migrated.paste_record.enabled);
        assert_eq!(migrated.paste_plain.binding.as_ref(), Some(&plain));

        legacy["preview"]["binding"] = serde_json::to_value(&paste).unwrap();
        legacy["pin"]["binding"] = serde_json::to_value(&plain).unwrap();
        let migrated = load(&legacy);
        assert_eq!(migrated.preview.binding, Some(paste));
        assert_eq!(migrated.pin.binding, Some(plain.clone()));
        assert_eq!(migrated.paste_record.binding, Some(old_key));
        assert!(migrated.paste_plain.binding.is_none());

        let mut custom = defaults;
        custom.set(PasteRecord, None, Some(false), &[]).unwrap();
        custom.set(PasteRecord, None, Some(true), &[]).unwrap();
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
        use ShortcutAction::{FocusSearch, OpenTranslation, Selection, ToggleHistory};
        let key = KeyBinding::new("KeyT", &["meta"]);
        let launch = TranslationShortcuts::defaults("meta");
        let mut records = ClipboardRecordShortcuts::default();
        assert!(records.uses_translate_launch_key());
        assert!(!launch.conflicts_with_records(&records));
        for action in [Selection, OpenTranslation] {
            assert!(!records.conflicts_with_translation(action, &key));
        }
        for action in [FocusSearch, ToggleHistory] {
            assert!(records.conflicts_with_translation(action, &key));
        }
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
            assert_eq!(normalize_binding(binding.clone()), Ok(binding));
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
    fn ignores_the_retired_record_list_focus_without_resetting_other_keys() {
        let mut old = serde_json::to_value(ClipboardRecordShortcuts::default()).unwrap();
        old["focusRecords"] = serde_json::json!({
            "binding":{"code":"F8","modifiers":[]},"enabled":true
        });
        old["navigateUp"] = serde_json::json!({"binding":null,"enabled":true});
        let restored = ClipboardRecordShortcuts::from_store(Some(old.to_string()));
        assert!(restored.navigate_up.binding.is_none());
        assert!(restored.navigate_down.binding.is_some());
        let saved = serde_json::to_value(&restored).unwrap();
        assert!(saved.get("focusRecords").is_none());
        assert!(serde_json::from_value::<ClipboardRecordAction>("focusRecords".into()).is_err());
    }

    #[test]
    fn shared_shortcuts_preserve_existing_bindings_and_reject_duplicate_tab() {
        use ClipboardRecordAction::*;
        let tab = KeyBinding::new("Tab", &[]);
        let mut old = serde_json::to_value(ClipboardRecordShortcuts::default()).unwrap();
        old.as_object_mut().unwrap().remove("copyRecord");
        old.as_object_mut().unwrap().remove("switchView");
        old["note"] =
            serde_json::to_value(ClipboardRecordShortcuts::default().copy_record).unwrap();
        let mut restored = ClipboardRecordShortcuts::from_store(Some(old.to_string()));
        assert!(restored.copy_record.binding.is_none());
        assert_eq!(restored.switch_view.binding, Some(tab.clone()));
        assert_eq!(serde_json::to_value(&restored.note).unwrap(), old["note"]);
        assert!(restored
            .set(CopyRecord, Some(tab.clone()), None, &[])
            .is_err());
        restored.set(SwitchView, Some(tab), None, &[]).unwrap();
        restored.set(SwitchView, None, Some(false), &[]).unwrap();
        let reset = restored.restore_clipboard().unwrap();
        assert!(!reset.switch_view.enabled);
        let saved = serde_json::to_value(&reset).unwrap();
        assert_eq!(saved["switchView"]["enabled"], false);
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
        shortcuts.set(PasteRecord, None, None, &[]).unwrap();
        for (action, code, modifiers) in [
            (NavigateUp, "KeyK", &[][..]),
            (NavigateDown, "Space", &[][..]),
            (PreviousCategory, "F6", &["shift"][..]),
            (NextCategory, "ShiftRight", &[][..]),
            (Preview, "Enter", &[][..]),
            (Delete, "Backspace", &["alt"][..]),
        ] {
            shortcuts
                .set(action, Some(KeyBinding::new(code, modifiers)), None, &[])
                .unwrap();
        }
        shortcuts.set(NavigateUp, None, Some(false), &[]).unwrap();
        let saved = serde_json::to_value(&shortcuts).unwrap();
        let restored = ClipboardRecordShortcuts::from_store(Some(saved.to_string()));
        assert_eq!(serde_json::to_value(&restored).unwrap(), saved);
        let reset = restored.restore_general().unwrap();
        assert_eq!(
            reset.navigate_up.binding,
            Some(KeyBinding::new("ArrowUp", &[]))
        );
        assert!(reset.navigate_up.enabled);
        assert_eq!(
            reset.preview.binding,
            ClipboardRecordShortcuts::default().preview.binding
        );
        assert!(reset.paste_record.binding.is_none());
        assert_eq!(
            restored.restore_clipboard().err().as_deref(),
            Some("shortcut_clipboard_panel")
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
    fn new_navigation_defaults_do_not_override_existing_bindings() {
        let mut old = serde_json::to_value(ClipboardRecordShortcuts::default()).unwrap();
        old.as_object_mut().unwrap().remove("navigateUp");
        old["preview"]["binding"] = serde_json::to_value(KeyBinding::new("ArrowUp", &[])).unwrap();
        let restored = ClipboardRecordShortcuts::from_store(Some(old.to_string()));
        assert!(restored.navigate_up.binding.is_none());
        assert_eq!(
            restored.preview.binding,
            Some(KeyBinding::new("ArrowUp", &[]))
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
            TranslationShortcuts::defaults("control").toggle_history,
            Some(KeyBinding::new("Slash", &["control"]))
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
        old.as_object_mut().unwrap().remove("toggleHistory");
        old["actions"] = serde_json::to_value(KeyBinding::new("Slash", &["meta"])).unwrap();
        let restored = load(&old, None, ClipboardRecordShortcuts::default());
        assert!(restored.focus_search.is_none());
        assert_eq!(
            restored.show_shortcuts,
            Some(KeyBinding::new("KeyK", &["meta"]))
        );
        assert_eq!(
            restored.toggle_history,
            Some(KeyBinding::new("Slash", &["meta"]))
        );
        let mut records = ClipboardRecordShortcuts::default();
        records.preview.binding = restored.toggle_history;
        assert!(load(&old, None, records).toggle_history.is_none());
    }

    #[test]
    fn swaps_old_help_and_history_defaults_without_replacing_custom_bindings() {
        let defaults = TranslationShortcuts::defaults("meta");
        let mut old = defaults.clone();
        std::mem::swap(&mut old.toggle_history, &mut old.show_shortcuts);
        let load = |stored: &serde_json::Value| {
            TranslationShortcuts::from_store(
                |key| (key == "translation_shortcuts").then(|| stored.to_string()),
                "meta",
            )
        };
        let stored = serde_json::to_value(&old).unwrap();
        assert_eq!(load(&stored), defaults);
        let mut legacy = stored;
        legacy.as_object_mut().unwrap().remove("toggleHistory");
        assert_eq!(load(&legacy), defaults);
        old.toggle_history = Some(KeyBinding::new("KeyJ", &["meta"]));
        assert_eq!(load(&serde_json::to_value(&old).unwrap()), old);
        old.toggle_history = None;
        assert_eq!(load(&serde_json::to_value(&old).unwrap()), old);
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
                ShortcutAction::ToggleHistory,
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
            ShortcutAction::ToggleHistory,
            None,
            Some(&clipboard_binding()),
        )
        .unwrap();
        assert_eq!(cleared.toggle_history, None);
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
