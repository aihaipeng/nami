use serde::{Deserialize, Serialize};

pub const CLIPBOARD_SHORTCUT_CODE: &str = "Backquote";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShortcutAction {
    Selection,
    Translate,
    CopyPrimary,
    CopySecondary,
    Pin,
    Actions,
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
        }
    }

    pub fn get(&self, action: ShortcutAction) -> Option<&KeyBinding> {
        match action {
            ShortcutAction::Selection => self.selection.as_ref(),
            ShortcutAction::Translate => self.translate.as_ref(),
            ShortcutAction::CopyPrimary => self.copy_primary.as_ref(),
            ShortcutAction::CopySecondary => self.copy_secondary.as_ref(),
            ShortcutAction::Pin => self.pin.as_ref(),
            ShortcutAction::Actions => self.actions.as_ref(),
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
        };
        *slot = binding;
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
    if modifiers.len() != binding.modifiers.len()
        || !modifiers
            .iter()
            .any(|modifier| matches!(modifier.as_str(), "meta" | "control" | "alt"))
    {
        return Err("shortcut_modifier".into());
    }
    Ok(KeyBinding { code, modifiers })
}

const TRANSLATION_ACTIONS: [ShortcutAction; 6] = [
    ShortcutAction::Selection,
    ShortcutAction::Translate,
    ShortcutAction::CopyPrimary,
    ShortcutAction::CopySecondary,
    ShortcutAction::Pin,
    ShortcutAction::Actions,
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
    let Ok(left) = normalize_binding(left.clone()) else {
        return false;
    };
    let Ok(right) = normalize_binding(right.clone()) else {
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
