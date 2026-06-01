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
                self.state.server.flash_attn = !self.state.server.flash_attn
            }
            SettingsField::Mlock => self.state.server.mlock = !self.state.server.mlock,
            SettingsField::Jinja => self.state.server.jinja = !self.state.server.jinja,
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
        let Some(value) = self.settings_edit_buffer.take() else {
            return;
        };
        match self.selected_settings_field {
            SettingsField::Host => self.state.server.host = value,
            SettingsField::Port => self.apply_parsed(value.parse::<u16>(), |server, value| {
                server.port = value;
            }),
            SettingsField::CtxSize => self.apply_parsed(value.parse::<u32>(), |server, value| {
                server.ctx_size = value;
            }),
            SettingsField::NGpuLayers => {
                self.apply_parsed(value.parse::<i32>(), |server, value| {
                    server.n_gpu_layers = value;
                })
            }
            SettingsField::BatchSize => self.apply_parsed(value.parse::<u32>(), |server, value| {
                server.batch_size = value;
            }),
            SettingsField::UBatchSize => {
                self.apply_parsed(value.parse::<u32>(), |server, value| {
                    server.ubatch_size = value;
                })
            }
            SettingsField::Threads => self.apply_parsed(value.parse::<usize>(), |server, value| {
                server.threads = value;
            }),
            SettingsField::FlashAttn => {
                self.apply_parsed(value.parse::<bool>(), |server, value| {
                    server.flash_attn = value;
                });
            }
            SettingsField::Mlock => {
                self.apply_parsed(value.parse::<bool>(), |server, value| {
                    server.mlock = value;
                });
            }
            SettingsField::ApiKey => self.state.server.api_key = value,
            SettingsField::Jinja => {
                self.apply_parsed(value.parse::<bool>(), |server, value| {
                    server.jinja = value;
                });
            }
            SettingsField::ChatTemplate => self.state.server.chat_template = value,
            SettingsField::ExtraArgs => {
                self.state.server.extra_args =
                    value.split_whitespace().map(ToOwned::to_owned).collect();
            }
        }
    }

    fn apply_parsed<T, E>(
        &mut self,
        parsed: Result<T, E>,
        apply: impl FnOnce(&mut lmml_state::ServerConfig, T),
    ) {
        match parsed {
            Ok(value) => apply(&mut self.state.server, value),
            Err(_error) => {
                self.status_message = "Invalid settings value".to_string();
            }
        }
    }
}
