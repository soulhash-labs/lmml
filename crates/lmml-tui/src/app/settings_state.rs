//! Settings tab state helpers for app orchestration.
//!
//! This module owns Settings field ordering, modal edit behavior, and
//! compatibility warning derivation while the parent [`super::App`] keeps the
//! high-level event dispatch.

use crossterm::event::{KeyCode, KeyEvent};
use lmml_models::ModelEntry;

use crate::action::Action;

use super::App;

/// Editable settings fields shown on the Settings tab.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingsField {
    /// Server host.
    Host,
    /// Server port.
    Port,
    /// Context size.
    CtxSize,
    /// GPU layers.
    NGpuLayers,
    /// Batch size.
    BatchSize,
    /// Micro-batch size.
    UBatchSize,
    /// Thread count.
    Threads,
    /// Flash attention toggle.
    FlashAttn,
    /// mlock toggle.
    Mlock,
    /// API key.
    ApiKey,
    /// Jinja toggle.
    Jinja,
    /// Chat template.
    ChatTemplate,
    /// Extra server args.
    ExtraArgs,
}

impl SettingsField {
    /// All editable fields in display order.
    pub const ALL: [SettingsField; 13] = [
        SettingsField::Host,
        SettingsField::Port,
        SettingsField::CtxSize,
        SettingsField::NGpuLayers,
        SettingsField::BatchSize,
        SettingsField::UBatchSize,
        SettingsField::Threads,
        SettingsField::FlashAttn,
        SettingsField::Mlock,
        SettingsField::ApiKey,
        SettingsField::Jinja,
        SettingsField::ChatTemplate,
        SettingsField::ExtraArgs,
    ];

    /// User-visible field name.
    pub fn label(self) -> &'static str {
        match self {
            SettingsField::Host => "host",
            SettingsField::Port => "port",
            SettingsField::CtxSize => "ctx_size",
            SettingsField::NGpuLayers => "n_gpu_layers",
            SettingsField::BatchSize => "batch_size",
            SettingsField::UBatchSize => "ubatch_size",
            SettingsField::Threads => "threads",
            SettingsField::FlashAttn => "flash_attn",
            SettingsField::Mlock => "mlock",
            SettingsField::ApiKey => "api_key",
            SettingsField::Jinja => "jinja",
            SettingsField::ChatTemplate => "chat_template",
            SettingsField::ExtraArgs => "extra_args",
        }
    }

    /// Whether the field is toggled directly instead of edited as text.
    pub fn is_bool(self) -> bool {
        matches!(
            self,
            SettingsField::FlashAttn | SettingsField::Mlock | SettingsField::Jinja
        )
    }

    fn index(self) -> usize {
        Self::ALL
            .iter()
            .position(|field| *field == self)
            .unwrap_or_default()
    }
}

/// Result of routing a key through Settings-specific handlers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SettingsKeyResult {
    /// Settings consumed the key and may emit an action.
    Handled(Option<Action>),
    /// Settings did not consume the key; global handling may continue.
    Unhandled,
}

impl App {
    /// Handle a key when the active tab is Settings.
    pub fn handle_settings_key(&mut self, key: KeyEvent) -> SettingsKeyResult {
        if self.settings_edit_buffer.is_some() {
            return SettingsKeyResult::Handled(match key.code {
                KeyCode::Esc => {
                    self.settings_edit_buffer = None;
                    None
                }
                KeyCode::Enter => {
                    self.apply_settings_edit();
                    None
                }
                KeyCode::Backspace => {
                    if let Some(buffer) = &mut self.settings_edit_buffer {
                        buffer.pop();
                    }
                    None
                }
                KeyCode::Char(ch) => {
                    if let Some(buffer) = &mut self.settings_edit_buffer {
                        buffer.push(ch);
                    }
                    None
                }
                KeyCode::Left
                | KeyCode::Right
                | KeyCode::Up
                | KeyCode::Down
                | KeyCode::Home
                | KeyCode::End
                | KeyCode::PageUp
                | KeyCode::PageDown
                | KeyCode::Tab
                | KeyCode::BackTab
                | KeyCode::Delete
                | KeyCode::Insert
                | KeyCode::F(_)
                | KeyCode::Null
                | KeyCode::CapsLock
                | KeyCode::ScrollLock
                | KeyCode::NumLock
                | KeyCode::PrintScreen
                | KeyCode::Pause
                | KeyCode::Menu
                | KeyCode::KeypadBegin
                | KeyCode::Media(_)
                | KeyCode::Modifier(_) => None,
            });
        }

        match key.code {
            KeyCode::Up => {
                self.previous_settings_field();
                SettingsKeyResult::Handled(None)
            }
            KeyCode::Down => {
                self.next_settings_field();
                SettingsKeyResult::Handled(None)
            }
            KeyCode::Esc | KeyCode::Backspace => SettingsKeyResult::Handled(None),
            KeyCode::Enter => SettingsKeyResult::Handled(Some(Action::SaveSettings)),
            KeyCode::Char(' ') => {
                self.toggle_settings_field();
                SettingsKeyResult::Handled(None)
            }
            KeyCode::Char('?') => SettingsKeyResult::Handled(Some(Action::ShowHelp)),
            KeyCode::Char('q') => SettingsKeyResult::Handled(Some(Action::Quit)),
            KeyCode::Char('e') => {
                self.settings_edit_buffer =
                    Some(self.settings_field_value(self.selected_settings_field));
                SettingsKeyResult::Handled(None)
            }
            KeyCode::Char('p') => SettingsKeyResult::Handled(Some(Action::ProbeServerCapabilities)),
            KeyCode::Char('s') => SettingsKeyResult::Handled(Some(Action::SaveSettings)),
            KeyCode::Char(_)
            | KeyCode::Left
            | KeyCode::Right
            | KeyCode::Home
            | KeyCode::End
            | KeyCode::PageUp
            | KeyCode::PageDown
            | KeyCode::Tab
            | KeyCode::BackTab
            | KeyCode::Delete
            | KeyCode::Insert
            | KeyCode::F(_)
            | KeyCode::Null
            | KeyCode::CapsLock
            | KeyCode::ScrollLock
            | KeyCode::NumLock
            | KeyCode::PrintScreen
            | KeyCode::Pause
            | KeyCode::Menu
            | KeyCode::KeypadBegin
            | KeyCode::Media(_)
            | KeyCode::Modifier(_) => SettingsKeyResult::Unhandled,
        }
    }

    /// Return warnings for the current server settings and probed binary.
    pub fn server_compat_warnings(&self) -> Vec<String> {
        let Some(caps) = &self.server_caps else {
            return Vec::new();
        };
        let model = self.selected_server_model().unwrap_or_else(|| ModelEntry {
            path: self.state.model.last_used.clone(),
            name: String::new(),
            size_bytes: 0,
            quant: String::new(),
            context_length: None,
            architecture: None,
            aliased: false,
        });
        lmml_compat::unsupported_warnings(&self.server_config(&model), caps)
            .into_iter()
            .map(|warning| warning.message)
            .collect()
    }

    /// String value for an editable Settings tab field.
    pub fn settings_field_value(&self, field: SettingsField) -> String {
        match field {
            SettingsField::Host => self.state.server.host.clone(),
            SettingsField::Port => self.state.server.port.to_string(),
            SettingsField::CtxSize => self.state.server.ctx_size.to_string(),
            SettingsField::NGpuLayers => self.state.server.n_gpu_layers.to_string(),
            SettingsField::BatchSize => self.state.server.batch_size.to_string(),
            SettingsField::UBatchSize => self.state.server.ubatch_size.to_string(),
            SettingsField::Threads => self.state.server.threads.to_string(),
            SettingsField::FlashAttn => self.state.server.flash_attn.to_string(),
            SettingsField::Mlock => self.state.server.mlock.to_string(),
            SettingsField::ApiKey => self.state.server.api_key.clone(),
            SettingsField::Jinja => self.state.server.jinja.to_string(),
            SettingsField::ChatTemplate => self.state.server.chat_template.clone(),
            SettingsField::ExtraArgs => self.state.server.extra_args.join(" "),
        }
    }

    /// User-visible value for a Settings tab field, masking secrets.
    pub fn settings_field_display_value(&self, field: SettingsField) -> String {
        match field {
            SettingsField::ApiKey if self.state.server.api_key.is_empty() => String::new(),
            SettingsField::ApiKey => "********".to_string(),
            _ => self.settings_field_value(field),
        }
    }

    fn next_settings_field(&mut self) {
        let next = (self.selected_settings_field.index() + 1) % SettingsField::ALL.len();
        self.selected_settings_field = SettingsField::ALL[next];
    }

    fn previous_settings_field(&mut self) {
        let current = self.selected_settings_field.index();
        let previous = if current == 0 {
            SettingsField::ALL.len() - 1
        } else {
            current - 1
        };
        self.selected_settings_field = SettingsField::ALL[previous];
    }

    fn toggle_settings_field(&mut self) {
        match self.selected_settings_field {
            SettingsField::FlashAttn => {
                self.state.server.flash_attn = !self.state.server.flash_attn;
                self.save_state_after("Settings updated");
            }
            SettingsField::Mlock => {
                self.state.server.mlock = !self.state.server.mlock;
                self.save_state_after("Settings updated");
            }
            SettingsField::Jinja => {
                self.state.server.jinja = !self.state.server.jinja;
                self.save_state_after("Settings updated");
            }
            SettingsField::Host
            | SettingsField::Port
            | SettingsField::CtxSize
            | SettingsField::NGpuLayers
            | SettingsField::BatchSize
            | SettingsField::UBatchSize
            | SettingsField::Threads
            | SettingsField::ApiKey
            | SettingsField::ChatTemplate
            | SettingsField::ExtraArgs => {}
        }
    }

    fn apply_settings_edit(&mut self) {
        let Some(value) = self.settings_edit_buffer.clone() else {
            return;
        };
        match validate_setting(self.selected_settings_field, &value) {
            Ok(validated) => {
                self.settings_edit_buffer = None;
                self.settings_validation_error = None;
                self.apply_validated_setting(validated);
                self.save_state_after("Settings updated");
            }
            Err(error) => {
                self.settings_validation_error = Some(error);
                self.status_message = "Invalid settings value".to_string();
            }
        }
    }

    fn apply_validated_setting(&mut self, setting: ValidatedSetting) {
        match setting {
            ValidatedSetting::Host(value) => self.state.server.host = value,
            ValidatedSetting::Port(value) => self.state.server.port = value,
            ValidatedSetting::CtxSize(value) => self.state.server.ctx_size = value,
            ValidatedSetting::NGpuLayers(value) => self.state.server.n_gpu_layers = value,
            ValidatedSetting::BatchSize(value) => self.state.server.batch_size = value,
            ValidatedSetting::UBatchSize(value) => self.state.server.ubatch_size = value,
            ValidatedSetting::Threads(value) => self.state.server.threads = value,
            ValidatedSetting::FlashAttn(value) => self.state.server.flash_attn = value,
            ValidatedSetting::Mlock(value) => self.state.server.mlock = value,
            ValidatedSetting::ApiKey(value) => self.state.server.api_key = value,
            ValidatedSetting::Jinja(value) => self.state.server.jinja = value,
            ValidatedSetting::ChatTemplate(value) => self.state.server.chat_template = value,
            ValidatedSetting::ExtraArgs(value) => self.state.server.extra_args = value,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ValidatedSetting {
    Host(String),
    Port(u16),
    CtxSize(u32),
    NGpuLayers(i32),
    BatchSize(u32),
    UBatchSize(u32),
    Threads(usize),
    FlashAttn(bool),
    Mlock(bool),
    ApiKey(String),
    Jinja(bool),
    ChatTemplate(String),
    ExtraArgs(Vec<String>),
}

fn validate_setting(field: SettingsField, value: &str) -> Result<ValidatedSetting, String> {
    match field {
        SettingsField::Host => validate_host(value).map(ValidatedSetting::Host),
        SettingsField::Port => {
            parse_u16_range(value, 1, u16::MAX, "port").map(ValidatedSetting::Port)
        }
        SettingsField::CtxSize => {
            parse_u32_range(value, 1, 1_048_576, "ctx_size").map(ValidatedSetting::CtxSize)
        }
        SettingsField::NGpuLayers => {
            parse_i32_range(value, -1, 999, "n_gpu_layers").map(ValidatedSetting::NGpuLayers)
        }
        SettingsField::BatchSize => {
            parse_u32_range(value, 1, 65_536, "batch_size").map(ValidatedSetting::BatchSize)
        }
        SettingsField::UBatchSize => {
            parse_u32_range(value, 1, 65_536, "ubatch_size").map(ValidatedSetting::UBatchSize)
        }
        SettingsField::Threads => {
            parse_usize_range(value, 1, 1024, "threads").map(ValidatedSetting::Threads)
        }
        SettingsField::FlashAttn => {
            parse_bool(value, "flash_attn").map(ValidatedSetting::FlashAttn)
        }
        SettingsField::Mlock => parse_bool(value, "mlock").map(ValidatedSetting::Mlock),
        SettingsField::ApiKey => Ok(ValidatedSetting::ApiKey(value.to_string())),
        SettingsField::Jinja => parse_bool(value, "jinja").map(ValidatedSetting::Jinja),
        SettingsField::ChatTemplate => Ok(ValidatedSetting::ChatTemplate(value.to_string())),
        SettingsField::ExtraArgs => split_shell_words(value).map(ValidatedSetting::ExtraArgs),
    }
}

fn validate_host(value: &str) -> Result<String, String> {
    let host = value.trim();
    if host.is_empty() {
        return Err("host is required".to_string());
    }
    if host.parse::<std::net::IpAddr>().is_ok() {
        return Ok(host.to_string());
    }
    if host.len() > 253 {
        return Err("host must be at most 253 characters".to_string());
    }
    for label in host.split('.') {
        if label.is_empty() || label.len() > 63 {
            return Err("host labels must be 1-63 characters".to_string());
        }
        let valid_chars = label
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '-');
        let valid_edges = label
            .as_bytes()
            .first()
            .zip(label.as_bytes().last())
            .map(|(first, last)| first.is_ascii_alphanumeric() && last.is_ascii_alphanumeric())
            .unwrap_or(false);
        if !valid_chars || !valid_edges {
            return Err("host must be a valid IP address or hostname".to_string());
        }
    }
    Ok(host.to_string())
}

fn parse_u16_range(value: &str, min: u16, max: u16, name: &str) -> Result<u16, String> {
    value
        .trim()
        .parse::<u16>()
        .map_err(|_| format!("{name} must be an integer"))
        .and_then(|parsed| {
            if (min..=max).contains(&parsed) {
                Ok(parsed)
            } else {
                Err(format!("{name} must be between {min} and {max}"))
            }
        })
}

fn parse_u32_range(value: &str, min: u32, max: u32, name: &str) -> Result<u32, String> {
    value
        .trim()
        .parse::<u32>()
        .map_err(|_| format!("{name} must be an integer"))
        .and_then(|parsed| {
            if (min..=max).contains(&parsed) {
                Ok(parsed)
            } else {
                Err(format!("{name} must be between {min} and {max}"))
            }
        })
}

fn parse_i32_range(value: &str, min: i32, max: i32, name: &str) -> Result<i32, String> {
    value
        .trim()
        .parse::<i32>()
        .map_err(|_| format!("{name} must be an integer"))
        .and_then(|parsed| {
            if (min..=max).contains(&parsed) {
                Ok(parsed)
            } else {
                Err(format!("{name} must be between {min} and {max}"))
            }
        })
}

fn parse_usize_range(value: &str, min: usize, max: usize, name: &str) -> Result<usize, String> {
    value
        .trim()
        .parse::<usize>()
        .map_err(|_| format!("{name} must be an integer"))
        .and_then(|parsed| {
            if (min..=max).contains(&parsed) {
                Ok(parsed)
            } else {
                Err(format!("{name} must be between {min} and {max}"))
            }
        })
}

fn parse_bool(value: &str, name: &str) -> Result<bool, String> {
    value
        .trim()
        .parse::<bool>()
        .map_err(|_| format!("{name} must be true or false"))
}

fn split_shell_words(value: &str) -> Result<Vec<String>, String> {
    let mut words = Vec::new();
    let mut current = String::new();
    let mut chars = value.chars().peekable();
    let mut quote: Option<char> = None;
    while let Some(ch) = chars.next() {
        match (quote, ch) {
            (None, '\'') | (None, '"') => quote = Some(ch),
            (Some(active), value) if value == active => quote = None,
            (None, ch) if ch.is_whitespace() => {
                if !current.is_empty() {
                    words.push(std::mem::take(&mut current));
                }
            }
            (_, '\\') => {
                if let Some(next) = chars.next() {
                    current.push(next);
                } else {
                    return Err("extra_args has a trailing escape".to_string());
                }
            }
            (_, ch) => current.push(ch),
        }
    }
    if quote.is_some() {
        return Err("extra_args has an unclosed quote".to_string());
    }
    if !current.is_empty() {
        words.push(current);
    }
    Ok(words)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_host_and_numeric_ranges() {
        assert!(validate_setting(SettingsField::Host, "127.0.0.1").is_ok());
        assert!(validate_setting(SettingsField::Host, "lmml.local").is_ok());
        assert!(validate_setting(SettingsField::Host, "-bad.local").is_err());
        assert!(validate_setting(SettingsField::Port, "0").is_err());
        assert!(validate_setting(SettingsField::Port, "65535").is_ok());
        assert!(validate_setting(SettingsField::CtxSize, "0").is_err());
        assert!(validate_setting(SettingsField::BatchSize, "65537").is_err());
        assert!(validate_setting(SettingsField::UBatchSize, "512").is_ok());
    }

    #[test]
    fn splits_shell_quoted_extra_args() {
        assert_eq!(
            split_shell_words("--flag 'two words' \"three words\" escaped\\ value")
                .expect("split args"),
            vec!["--flag", "two words", "three words", "escaped value"]
        );
        assert!(split_shell_words("--bad 'open").is_err());
        assert!(split_shell_words("--bad \\").is_err());
    }

    #[test]
    fn masks_api_key_for_display() {
        let mut app = App::default();
        app.state.server.api_key = "secret".to_string();
        assert_eq!(
            app.settings_field_display_value(SettingsField::ApiKey),
            "********"
        );
        assert_eq!(app.settings_field_value(SettingsField::ApiKey), "secret");
    }
}
