// Keybinding configuration
// Future: Allow users to customize keybindings

use crossterm::event::{KeyCode, KeyModifiers};

#[derive(Debug, Clone)]
pub struct KeyBinding {
    pub code: KeyCode,
    pub modifiers: KeyModifiers,
    pub description: &'static str,
}

pub fn default_keybindings() -> Vec<KeyBinding> {
    vec![
        KeyBinding {
            code: KeyCode::Char('q'),
            modifiers: KeyModifiers::NONE,
            description: "Quit (Normal mode)",
        },
        KeyBinding {
            code: KeyCode::Char('i'),
            modifiers: KeyModifiers::NONE,
            description: "Insert mode",
        },
        KeyBinding {
            code: KeyCode::Char('s'),
            modifiers: KeyModifiers::NONE,
            description: "Schema browser",
        },
        KeyBinding {
            code: KeyCode::Enter,
            modifiers: KeyModifiers::NONE,
            description: "Execute query",
        },
        // Add more...
    ]
}
