//! Settings → Notifications: independently configurable session chimes plus
//! desktop banners on the same status transitions (`shell::on_state_changed`).
//! Each chime can be swapped for a user-chosen audio file and auditioned
//! with its banner from the row.
//!
//! The ShortcutsPage arrangement: the page holds a working copy, every flip
//! emits [`NotificationsEvent::Changed`], and the shell persists it. Nothing
//! here talks RPC — all preferences are device-local UI settings.

use gpui::{Context, EventEmitter, SharedString, Window, div, prelude::*, px};

use crate::icons;
use crate::popover;
use crate::settings::widgets;
use crate::sound::{CustomSounds, Sound};
use crate::theme::Theme;

#[derive(Debug, Clone)]
pub enum NotificationsEvent {
    /// A toggle flipped — persist the complete notification preference set.
    Changed {
        sound: bool,
        completion_sound: bool,
        input_sound: bool,
        attention_sound: bool,
        custom_sounds: CustomSounds,
        desktop: bool,
        background_only: bool,
        agent_updates: bool,
    },
}

pub struct NotificationsPage {
    scroll: widgets::PageScroll,
    sound: bool,
    completion_sound: bool,
    input_sound: bool,
    attention_sound: bool,
    custom_sounds: CustomSounds,
    /// Why the last chosen or tested sound file was rejected.
    sound_error: Option<SharedString>,
    desktop: bool,
    background_only: bool,
    agent_updates: bool,
}

impl EventEmitter<NotificationsEvent> for NotificationsPage {}

#[derive(Clone, Copy)]
enum NotificationPreference {
    Sound,
    CompletionSound,
    InputSound,
    AttentionSound,
    Desktop,
    BackgroundOnly,
    AgentUpdates,
}

fn is_switch_activation(key: &str, is_held: bool) -> bool {
    !is_held && matches!(key, "enter" | "space")
}

/// Stable element-id fragment for a chime's row controls.
fn sound_key(sound: Sound) -> &'static str {
    match sound {
        Sound::Done => "completion",
        Sound::Request => "input",
        Sound::Attention => "attention",
    }
}

fn sound_label(sound: Sound) -> &'static str {
    match sound {
        Sound::Done => "Task completed",
        Sound::Request => "Input required",
        Sound::Attention => "Errors and disconnections",
    }
}

/// The row's meta line: the custom file's name, or the embedded default.
fn sound_file_label(custom: Option<&std::path::Path>) -> SharedString {
    match custom {
        Some(path) => path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.display().to_string())
            .into(),
        None => "Default chime".into(),
    }
}

fn unsupported_sound_message() -> SharedString {
    let extensions = crate::sound::CUSTOM_SOUND_EXTENSIONS
        .iter()
        .map(|ext| ext.to_ascii_uppercase())
        .collect::<Vec<_>>()
        .join(", ");
    format!("That file can't be used as a sound. Choose one of: {extensions}.").into()
}

fn interactive_switch(
    element: gpui::Stateful<gpui::Div>,
    accent: gpui::Hsla,
    preference: NotificationPreference,
    cx: &mut Context<NotificationsPage>,
) -> gpui::Stateful<gpui::Div> {
    element
        .tab_index(0)
        .focus_visible(move |style| style.border_2().border_color(accent))
        .cursor_pointer()
        .on_click(cx.listener(move |this, _, _, cx| {
            this.toggle(preference, cx);
        }))
        .on_key_down(cx.listener(move |this, event: &gpui::KeyDownEvent, _, cx| {
            if is_switch_activation(&event.keystroke.key, event.is_held) {
                cx.stop_propagation();
                this.toggle(preference, cx);
            }
        }))
}

impl NotificationsPage {
    pub fn new(
        sound: bool,
        completion_sound: bool,
        input_sound: bool,
        attention_sound: bool,
        custom_sounds: CustomSounds,
        desktop: bool,
        background_only: bool,
        agent_updates: bool,
        _cx: &mut Context<Self>,
    ) -> Self {
        Self {
            scroll: widgets::PageScroll::default(),
            sound,
            completion_sound,
            input_sound,
            attention_sound,
            custom_sounds,
            sound_error: None,
            desktop,
            background_only,
            agent_updates,
        }
    }

    fn emit(&self, cx: &mut Context<Self>) {
        cx.emit(NotificationsEvent::Changed {
            sound: self.sound,
            completion_sound: self.completion_sound,
            input_sound: self.input_sound,
            attention_sound: self.attention_sound,
            custom_sounds: self.custom_sounds.clone(),
            desktop: self.desktop,
            background_only: self.background_only,
            agent_updates: self.agent_updates,
        });
    }

    fn toggle(&mut self, preference: NotificationPreference, cx: &mut Context<Self>) {
        let value = match preference {
            NotificationPreference::Sound => &mut self.sound,
            NotificationPreference::CompletionSound => &mut self.completion_sound,
            NotificationPreference::InputSound => &mut self.input_sound,
            NotificationPreference::AttentionSound => &mut self.attention_sound,
            NotificationPreference::Desktop => &mut self.desktop,
            NotificationPreference::BackgroundOnly => &mut self.background_only,
            NotificationPreference::AgentUpdates => &mut self.agent_updates,
        };
        *value = !*value;
        self.emit(cx);
        cx.notify();
    }

    fn set_custom_sound(
        &mut self,
        sound: Sound,
        path: Option<std::path::PathBuf>,
        cx: &mut Context<Self>,
    ) {
        if path
            .as_deref()
            .is_some_and(|path| !crate::sound::is_supported_custom_sound(path))
        {
            self.sound_error = Some(unsupported_sound_message());
        } else {
            self.sound_error = None;
            self.custom_sounds.set(sound, path);
            self.emit(cx);
        }
        cx.notify();
    }

    fn choose_sound(&mut self, sound: Sound, cx: &mut Context<Self>) {
        let receiver = cx.prompt_for_paths(gpui::PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some("Choose Sound".into()),
        });
        cx.spawn(async move |this, cx| {
            let path = match receiver.await {
                Ok(Ok(Some(mut paths))) => paths.pop(),
                _ => None,
            };
            let Some(path) = path else {
                return;
            };
            let _ = this.update(cx, |page, cx| page.set_custom_sound(sound, Some(path), cx));
        })
        .detach();
    }

    /// Audition one event exactly as a session would deliver it: its chime,
    /// plus its banner when banners are on. Deliberate, so neither the
    /// per-event mute nor the background-only rule applies.
    fn test_sound(&mut self, sound: Sound, cx: &mut Context<Self>) {
        let custom = self.custom_sounds.get(sound);
        self.sound_error = custom.filter(|path| !path.is_file()).map(|path| {
            format!(
                "{} can't be found. Playing the default chime instead.",
                path.display()
            )
            .into()
        });
        crate::sound::play(sound, custom);
        if self.desktop {
            crate::notify::post("Test notification", sound.banner_body(), None);
        }
        cx.notify();
    }

    /// Test / Choose / Reset for one chime's row; inert while the master
    /// switch is off, like the row's own toggle.
    fn sound_actions(&self, sound: Sound, theme: &Theme, cx: &mut Context<Self>) -> gpui::Div {
        let accent = theme.accent;
        let interactive = self.sound;
        let key = sound_key(sound);
        let label = sound_label(sound);
        let button = |action: &'static str, tone: widgets::ActionTone| {
            widgets::text_action(theme, tone, action)
                .id(SharedString::from(format!(
                    "notifications-{key}-sound-{}",
                    action.to_ascii_lowercase()
                )))
                .role(gpui::Role::Button)
                .aria_label(SharedString::from(format!("{action} {label} sound")))
                .when(interactive, |el| {
                    el.tab_index(0)
                        .focus_visible(move |s| s.border_2().border_color(accent))
                })
        };
        div()
            .flex_none()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(4.0))
            .child(
                button("Test", widgets::ActionTone::Quiet).when(interactive, |el| {
                    el.on_click(cx.listener(move |this, _, _, cx| this.test_sound(sound, cx)))
                }),
            )
            .child(
                button("Choose", widgets::ActionTone::Outlined).when(interactive, |el| {
                    el.on_click(cx.listener(move |this, _, _, cx| this.choose_sound(sound, cx)))
                }),
            )
            .when(self.custom_sounds.get(sound).is_some(), |el| {
                el.child(
                    button("Reset", widgets::ActionTone::Quiet).when(interactive, |el| {
                        el.on_click(
                            cx.listener(move |this, _, _, cx| {
                                this.set_custom_sound(sound, None, cx)
                            }),
                        )
                    }),
                )
            })
    }

    fn on_scroll_hovered(&mut self, hovered: &bool, _: &mut Window, cx: &mut Context<Self>) {
        if self.scroll.set_list_hovered(*hovered) {
            cx.notify();
        }
    }
}

impl popover::ScrollRailHost for NotificationsPage {
    fn rail_bar(&mut self) -> &mut popover::MenuScrollbarState {
        self.scroll.rail_bar()
    }

    fn rail_scroll(&self) -> Option<gpui::ScrollHandle> {
        self.scroll.rail_scroll()
    }
}

impl Render for NotificationsPage {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::of(cx).for_settings_surface();
        let accent = theme.accent;
        let sound = self.sound;
        let completion_sound = self.completion_sound;
        let input_sound = self.input_sound;
        let attention_sound = self.attention_sound;
        let desktop = self.desktop;
        let background_only = self.background_only;
        let agent_updates = self.agent_updates;
        let toggle = |id: &'static str, label: &'static str, enabled: bool, interactive: bool| {
            // Keep the visual inside its 56×40 activation target.
            // Disabled subordinate controls remain named switches in the
            // accessibility tree, but have no focus or input handlers.
            div()
                .id(id)
                .flex_none()
                .w(px(widgets::SWITCH_WIDTH))
                .h(px(40.0))
                .flex()
                .items_center()
                .justify_center()
                .role(gpui::Role::Switch)
                .aria_label(label)
                .aria_toggled(if enabled {
                    gpui::Toggled::True
                } else {
                    gpui::Toggled::False
                })
                .when(!interactive, |el| {
                    el.aria_description("Unavailable while its parent setting is off")
                })
                .child(widgets::toggle_switch(&theme, enabled, id))
        };
        let sound_file_line = |sound: Sound| {
            let name = sound_file_label(self.custom_sounds.get(sound));
            widgets::meta_line(&theme, vec![div().child(name).into_any_element()])
        };
        let card = widgets::section_card(&theme)
            .mt_0()
            .child(
                widgets::card_row(&theme, true)
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(160.0))
                            .flex()
                            .flex_col()
                            .child(widgets::row_title(&theme, "Session sounds")),
                    )
                    .child(interactive_switch(
                        toggle("notifications-sound-toggle", "Session sounds", sound, true),
                        accent,
                        NotificationPreference::Sound,
                        cx,
                    )),
            )
            .child(
                widgets::card_row(&theme, false)
                    .when(!sound, |el| el.opacity(0.55))
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(160.0))
                            .flex()
                            .flex_col()
                            .child(widgets::row_title(&theme, "Task completed"))
                            .child(sound_file_line(Sound::Done)),
                    )
                    .child(self.sound_actions(Sound::Done, &theme, cx))
                    .child(
                        toggle(
                            "notifications-completion-sound-toggle",
                            "Task completed sound",
                            completion_sound,
                            sound,
                        )
                        .when(sound, |el| {
                            interactive_switch(
                                el,
                                accent,
                                NotificationPreference::CompletionSound,
                                cx,
                            )
                        }),
                    ),
            )
            .child(
                widgets::card_row(&theme, false)
                    .when(!sound, |el| el.opacity(0.55))
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(160.0))
                            .flex()
                            .flex_col()
                            .child(widgets::row_title(&theme, "Input required"))
                            .child(sound_file_line(Sound::Request)),
                    )
                    .child(self.sound_actions(Sound::Request, &theme, cx))
                    .child(
                        toggle(
                            "notifications-input-sound-toggle",
                            "Input required sound",
                            input_sound,
                            sound,
                        )
                        .when(sound, |el| {
                            interactive_switch(el, accent, NotificationPreference::InputSound, cx)
                        }),
                    ),
            )
            .child(
                widgets::card_row(&theme, false)
                    .when(!sound, |el| el.opacity(0.55))
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(160.0))
                            .flex()
                            .flex_col()
                            .child(widgets::row_title(&theme, "Errors and disconnections"))
                            .child(sound_file_line(Sound::Attention)),
                    )
                    .child(self.sound_actions(Sound::Attention, &theme, cx))
                    .child(
                        toggle(
                            "notifications-attention-sound-toggle",
                            "Errors and disconnections sound",
                            attention_sound,
                            sound,
                        )
                        .when(sound, |el| {
                            interactive_switch(
                                el,
                                accent,
                                NotificationPreference::AttentionSound,
                                cx,
                            )
                        }),
                    ),
            );
        let desktop_card = widgets::section_card(&theme)
            .mt_0()
            .child(
                widgets::card_row(&theme, true)
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(160.0))
                            .flex()
                            .flex_col()
                            .child(widgets::row_title(&theme, "Desktop notifications")),
                    )
                    .child(interactive_switch(
                        toggle(
                            "notifications-desktop-toggle",
                            "Desktop notifications",
                            desktop,
                            true,
                        ),
                        accent,
                        NotificationPreference::Desktop,
                        cx,
                    )),
            )
            .child(
                widgets::card_row(&theme, false)
                    .when(!desktop, |el| el.opacity(0.55))
                    .child(widgets::row_tile(&theme, icons::REFRESH))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .child(widgets::row_title(&theme, "Agent updates"))
                            .child(widgets::meta_line(
                                &theme,
                                vec![
                                    div()
                                        .child(SharedString::from(
                                            "Show a banner when monitored agent CLIs have updates.",
                                        ))
                                        .into_any_element(),
                                ],
                            )),
                    )
                    .child(
                        toggle(
                            "notifications-agent-updates-toggle",
                            "Agent update notifications",
                            agent_updates,
                            desktop,
                        )
                        .when(desktop, |el| {
                            interactive_switch(el, accent, NotificationPreference::AgentUpdates, cx)
                        }),
                    ),
            )
            .child(
                // Sub-option of the banner row: dimmed + inert while banners
                // are off (the harnesses not-installed treatment).
                widgets::card_row(&theme, false)
                    .when(!desktop, |el| el.opacity(0.55))
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(160.0))
                            .flex()
                            .flex_col()
                            .child(widgets::row_title(&theme, "Only when in the background")),
                    )
                    .child(
                        toggle(
                            "notifications-background-toggle",
                            "Only notify when Zeron is in the background",
                            background_only,
                            desktop,
                        )
                        .when(desktop, |el| {
                            interactive_switch(
                                el,
                                accent,
                                NotificationPreference::BackgroundOnly,
                                cx,
                            )
                        }),
                    ),
            );

        let scrollbar = popover::rail(self, "notifications-page-scrollbar", &theme, cx);
        div()
            .id("notifications-page-host")
            .relative()
            .size_full()
            .on_hover(cx.listener(Self::on_scroll_hovered))
            .child(
                crate::edge_fade::edge_faded(
                    16.0,
                    true,
                    true,
                    div()
                        .id("notifications-page")
                        .size_full()
                        .overflow_y_scroll()
                        .track_scroll(&self.scroll.scroll)
                        .child(
                            widgets::page_column()
                                .child(widgets::page_header(&theme, "Notifications", None))
                                .child(
                                    widgets::section(&theme, "Desktop", desktop_card).mt(px(24.0)),
                                )
                                .child(widgets::section(&theme, "Sounds", card))
                                .children(
                                    self.sound_error
                                        .clone()
                                        .map(|error| widgets::error_strip(&theme, error)),
                                ),
                        ),
                )
                .fade_overflow_y(&self.scroll.scroll),
            )
            .children(scrollbar)
    }
}

#[cfg(test)]
mod tests {
    use super::is_switch_activation;

    #[test]
    fn sound_rows_name_the_custom_file_or_the_default() {
        assert_eq!(super::sound_file_label(None).as_ref(), "Default chime");
        assert_eq!(
            super::sound_file_label(Some(std::path::Path::new("/tmp/sounds/ding.wav"))).as_ref(),
            "ding.wav"
        );
    }

    #[test]
    fn switches_accept_enter_or_space_once_per_press() {
        assert!(is_switch_activation("enter", false));
        assert!(is_switch_activation("space", false));
        assert!(!is_switch_activation("escape", false));
        assert!(!is_switch_activation("space", true));
    }
}
