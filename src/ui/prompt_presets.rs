use glam::Vec2;
use mega_ui::{ScrollAxes, TextStyle, Ui};

use crate::prompt_preset::{PromptKind, PromptPreset};

enum View {
    List,
    Name,
}

#[cfg(test)]
mod tests {
    use super::*;
    use mega_ui::{Rect, UiInput, UiOutput};

    struct Harness {
        ui: Ui,
        panel: PromptPresets,
        items: Vec<PromptPreset>,
        positive: String,
        negative: String,
        dirty: bool,
        kind: PromptKind,
    }

    impl Harness {
        fn new() -> Self {
            let mut ui = Ui::new();
            ui.load_builtin_icons();
            let mut panel = PromptPresets::default();
            panel.open = true;
            Self {
                ui,
                panel,
                items: Vec::new(),
                positive: String::new(),
                negative: String::new(),
                dirty: false,
                kind: PromptKind::Image,
            }
        }

        fn frame(&mut self, input: UiInput) -> UiOutput {
            self.ui.begin_frame(UiInput {
                viewport: Vec2::new(440.0, 600.0),
                ..input
            });
            self.dirty |= self.panel.draw(
                &mut self.ui,
                "Prompt",
                self.kind,
                &mut self.items,
                &mut self.positive,
                (self.kind == PromptKind::Image).then_some(&mut self.negative),
            );
            self.ui.end_frame()
        }

        fn row(&mut self) -> Rect {
            let mut output = self.frame(UiInput::default());
            for _ in 0..3 {
                output = self.frame(UiInput::default());
            }
            output
                .draw_list
                .iter()
                .find(|command| {
                    command.kind == 2.0
                        && command.rect.height() > 40.0
                        && command.rect.height() < 100.0
                        && command.rect.width() > 150.0
                })
                .expect("preset row")
                .rect
        }

        fn click(&mut self, mouse_pos: Vec2) {
            self.frame(UiInput {
                mouse_pos,
                mouse_pressed: true,
                mouse_down: true,
                ..Default::default()
            });
            self.frame(UiInput {
                mouse_pos,
                mouse_released: true,
                ..Default::default()
            });
        }
    }

    #[test]
    fn choosing_preset_applies_both_prompts_without_dirtying_project() {
        let mut h = Harness::new();
        h.items.push(PromptPreset::new(
            "My style".into(),
            "positive text".into(),
            "negative text".into(),
        ));
        let row = h.row();
        h.click((row.min + row.max) * 0.5);
        assert_eq!(h.positive, "positive text");
        assert_eq!(h.negative, "negative text");
        assert!(!h.panel.open && !h.dirty);
    }

    #[test]
    fn libraries_filter_apply_and_delete_using_original_indices() {
        for kind in [PromptKind::Image, PromptKind::Video] {
            let mut h = Harness::new();
            h.kind = kind;
            let other = if kind == PromptKind::Image {
                PromptKind::Video
            } else {
                PromptKind::Image
            };
            h.items.push(
                PromptPreset::new("Hidden".into(), "hidden".into(), String::new()).with_kind(other),
            );
            h.items.push(
                PromptPreset::new("Visible".into(), "visible".into(), String::new())
                    .with_kind(kind),
            );
            let row = h.row();
            h.click((row.min + row.max) * 0.5);
            assert_eq!(h.positive, "visible");
            assert!(!h.dirty);
            h.panel.open = true;
            let row = h.row();
            h.click(Vec2::new(row.max.x + 16.0, row.min.y + 12.0));
            assert_eq!(h.items.len(), 1);
            assert_eq!(h.items[0].kind, other);
            assert_eq!(h.items[0].positive, "hidden");
            assert!(h.dirty);
        }
    }

    #[test]
    fn saving_video_preset_assigns_video_kind_without_negative_prompt() {
        let mut h = Harness::new();
        h.kind = PromptKind::Video;
        h.panel.view = View::Name;
        h.panel.name = "Camera move".into();
        h.positive = "Orbit <Picture 1> beside <Video 2>".into();
        h.negative = "Image negative prompt".into();
        let output = h.frame(UiInput::default());
        let button = output
            .draw_list
            .iter()
            .rev()
            .find(|c| c.kind == 2.0 && c.rect.height() > 24.0 && c.rect.height() <= 28.0)
            .unwrap()
            .rect;
        h.click((button.min + button.max) * 0.5);
        assert_eq!(h.items.len(), 1);
        assert_eq!(h.items[0].kind, PromptKind::Video);
        assert_eq!(h.items[0].positive, h.positive);
        assert!(h.items[0].negative.is_empty());
        assert!(h.dirty && !h.panel.open);
    }

    #[test]
    fn delete_icon_marks_project_dirty_and_keeps_current_prompt() {
        let mut h = Harness::new();
        h.positive = "Keep this".into();
        h.items.push(PromptPreset::new(
            "First".into(),
            "one".into(),
            String::new(),
        ));
        h.items.push(PromptPreset::new(
            "Second".into(),
            "two".into(),
            String::new(),
        ));
        let row = h.row();
        h.click(Vec2::new(row.max.x + 16.0, row.min.y + 12.0));
        assert_eq!(h.items.len(), 1);
        assert_eq!(h.items[0].name, "Second");
        assert_eq!(h.positive, "Keep this");
        assert!(h.dirty && h.panel.open);
    }

    #[test]
    fn saving_named_preset_updates_project_data_and_dirty_flag() {
        let mut h = Harness::new();
        h.panel.view = View::Name;
        h.panel.name = "My preset".into();
        h.positive = "positive".into();
        h.negative = "negative".into();
        let output = h.frame(UiInput::default());
        let button = output
            .draw_list
            .iter()
            .rev()
            .find(|c| c.kind == 2.0 && c.rect.height() > 24.0 && c.rect.height() <= 28.0)
            .unwrap()
            .rect;
        h.click((button.min + button.max) * 0.5);
        assert_eq!(h.items.len(), 1);
        assert!(!h.items[0].id.is_empty());
        assert_eq!(h.items[0].name, "My preset");
        assert_eq!(h.items[0].positive, "positive");
        assert_eq!(h.items[0].negative, "negative");
        assert!(h.dirty && !h.panel.open);
        assert!(Harness::new().items.is_empty());
    }
}

pub struct PromptPresets {
    pub open: bool,
    query: String,
    name: String,
    view: View,
    focus_requested: bool,
}

impl Default for PromptPresets {
    fn default() -> Self {
        Self {
            open: false,
            query: String::new(),
            name: String::new(),
            view: View::List,
            focus_requested: false,
        }
    }
}

enum Action {
    Apply(usize),
    Delete(usize),
    Save,
}

impl PromptPresets {
    pub fn close(&mut self) {
        self.open = false;
        self.view = View::List;
        self.query.clear();
    }

    pub fn draw(
        &mut self,
        ui: &mut Ui,
        label: &str,
        kind: PromptKind,
        items: &mut Vec<PromptPreset>,
        positive: &mut String,
        mut negative: Option<&mut String>,
    ) -> bool {
        let mut changed = false;
        let was_open = self.open;
        let mut open = self.open;
        let mut action = None;
        ui.horizontal(|ui| {
            ui.label(label);
            ui.spacer();

            let height = match self.view {
                View::List => 330.0,
                View::Name => 165.0,
            };
            let result = ui.popover_button(
                "prompt_presets",
                "Presets",
                &mut open,
                Vec2::new(340.0, height),
                |ui| {
                    if !was_open {
                        self.focus_requested = true;
                    }
                    self.contents(ui, items, kind, !positive.trim().is_empty())
                },
            );
            action = result.flatten();
        });
        self.open = open;
        if let Some(action) = action {
            match action {
                Action::Apply(index) => {
                    let preset = &items[index];
                    *positive = preset.positive.clone();
                    if let Some(negative) = &mut negative {
                        **negative = preset.negative.clone();
                    }
                    self.close();
                }
                Action::Delete(index) => {
                    items.remove(index);
                    changed = true;
                    self.view = View::List;
                }
                Action::Save => {
                    items.push(
                        PromptPreset::new(
                            self.name.trim().to_string(),
                            positive.clone(),
                            negative.as_deref().cloned().unwrap_or_default(),
                        )
                        .with_kind(kind),
                    );
                    changed = true;
                    self.close();
                }
            }
        }
        if !self.open {
            self.view = View::List;
            self.query.clear();
        }
        changed
    }

    fn contents(
        &mut self,
        ui: &mut Ui,
        items: &[PromptPreset],
        kind: PromptKind,
        can_save: bool,
    ) -> Option<Action> {
        match self.view {
            View::List => {
                ui.label("Search");
                if self.focus_requested {
                    ui.focus_widget("search");
                    self.focus_requested = false;
                }
                ui.text_input("search", &mut self.query);
                ui.separator();
                let query = self.query.trim().to_lowercase();
                let mut action = None;
                let size = Vec2::new(
                    ui.available_size().x,
                    (ui.available_size().y - 55.0 * ui.scale()).max(30.0 * ui.scale()),
                );
                ui.scroll_area("preset_list", size, ScrollAxes::Vertical, |ui| {
                    let mut found = false;
                    for (index, preset) in items.iter().enumerate() {
                        if preset.kind != kind {
                            continue;
                        }
                        if !preset.name.to_lowercase().contains(&query)
                            && !preset.positive.to_lowercase().contains(&query)
                        {
                            continue;
                        }
                        found = true;
                        ui.horizontal(|ui| {
                            let row_width =
                                (ui.available_size().x - 30.0 * ui.scale()).max(40.0 * ui.scale());
                            ui.flex_width(row_width, |ui| {
                                let width = (ui.available_size().x / ui.scale() - 20.0).max(20.0);
                                if ui
                                    .button_with(&format!("preset_{index}"), |ui| {
                                        ui.column(|ui| {
                                            ui.label_fixed(width, &preset.name);
                                            let text = preset.positive.replace(['\n', '\r'], " ");
                                            let limit = (width / 7.0).max(8.0) as usize;
                                            let mut excerpt: String =
                                                text.chars().take(limit).collect();
                                            if text.chars().count() > limit {
                                                excerpt.push_str("...");
                                            }
                                            ui.label_styled(
                                                &excerpt,
                                                TextStyle {
                                                    color: [0.65, 0.65, 0.65, 1.0],
                                                    size: 12.0,
                                                },
                                            );
                                        });
                                    })
                                    .clicked()
                                {
                                    action = Some(Action::Apply(index));
                                }
                            });
                            if ui.icon_button(&format!("preset_delete_{index}"), "delete", false) {
                                action = Some(Action::Delete(index));
                            }
                        });
                    }
                    if !found {
                        ui.label("No prompts");
                    }
                });
                ui.separator();
                ui.add_enabled(can_save, |ui| {
                    if ui.button("save_current", "Save current...").clicked() {
                        self.name.clear();
                        self.view = View::Name;
                        self.focus_requested = true;
                    }
                });
                action
            }
            View::Name => {
                if ui.button("back", "Back").clicked() {
                    self.view = View::List;
                }
                ui.label("Save prompt");
                if self.focus_requested {
                    ui.focus_widget("name");
                    self.focus_requested = false;
                }
                ui.text_input("name", &mut self.name);
                let mut action = None;
                ui.add_enabled(!self.name.trim().is_empty(), |ui| {
                    if ui.button("save", "Save").clicked() {
                        action = Some(Action::Save);
                    }
                });
                action
            }
        }
    }
}
