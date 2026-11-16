//! Editor-owned numeric slider, built entirely from mega-ui's public primitives.
use glam::Vec2;
use mega_ui::{CursorIcon, Rect, Response, Ui};

use crate::host::FrameInput;

pub struct ValueSlider<'a> {
    label: &'a str,
    min: f32,
    max: f32,
    step: f32,
    multiplier: f32,
    suffix: &'a str,
}

impl<'a> ValueSlider<'a> {
    pub fn new(label: &'a str, min: f32, max: f32, step: f32, suffix: &'a str) -> Self {
        Self {
            label,
            min,
            max,
            step,
            multiplier: 1.0,
            suffix,
        }
    }

    pub fn percent(label: &'a str, min: f32) -> Self {
        Self {
            multiplier: 100.0,
            ..Self::new(label, min, 1.0, 0.01, "%")
        }
    }

    fn format(&self, value: f32) -> String {
        let text = format!("{:.2}", value * self.multiplier);
        text.trim_end_matches('0').trim_end_matches('.').to_string()
    }

    fn parse(&self, text: &str) -> Option<f32> {
        let value = text.trim().replace(',', ".").parse::<f32>().ok()? / self.multiplier;
        value.is_finite().then(|| value.clamp(self.min, self.max))
    }
}

struct Edit {
    id: String,
    text: String,
    caret: usize,
    anchor: usize,
}

impl Edit {
    fn selection(&self) -> std::ops::Range<usize> {
        self.caret.min(self.anchor)..self.caret.max(self.anchor)
    }

    fn insert(&mut self, text: &str) {
        let filtered: String = text
            .chars()
            .filter(|c| c.is_ascii_digit() || ".,+-eE".contains(*c))
            .map(|c| if c == ',' { '.' } else { c })
            .take(48)
            .collect();
        if filtered.is_empty() {
            return;
        }
        let range = self.selection();
        if self.text.len() - range.len() + filtered.len() > 48 {
            return;
        }
        self.text.replace_range(range.clone(), &filtered);
        self.caret = range.start + filtered.len();
        self.anchor = self.caret;
    }

    fn input(&mut self, input: &FrameInput) {
        if input.key_select_all {
            self.anchor = 0;
            self.caret = self.text.len();
        }
        let next = if input.key_home {
            Some(0)
        } else if input.key_end {
            Some(self.text.len())
        } else if input.key_left {
            Some(if !input.key_shift && !self.selection().is_empty() {
                self.selection().start
            } else {
                self.caret.saturating_sub(1)
            })
        } else if input.key_right {
            Some(if !input.key_shift && !self.selection().is_empty() {
                self.selection().end
            } else {
                (self.caret + 1).min(self.text.len())
            })
        } else {
            None
        };
        if let Some(next) = next {
            self.caret = next;
            if !input.key_shift {
                self.anchor = next;
            }
        }
        if input.key_backspace || input.key_delete {
            let mut range = self.selection();
            if range.is_empty() {
                if input.key_backspace {
                    range.start = range.start.saturating_sub(1);
                } else {
                    range.end = (range.end + 1).min(self.text.len());
                }
            }
            self.text.replace_range(range.clone(), "");
            self.caret = range.start;
            self.anchor = self.caret;
        }
        if input.key_paste {
            if let Ok(text) = arboard::Clipboard::new().and_then(|mut c| c.get_text()) {
                self.insert(&text);
            }
        } else if !input.key_ctrl {
            self.insert(&input.text);
        }
    }
}

#[derive(Default)]
pub struct ValueSliderState {
    edit: Option<Edit>,
    drag: Option<String>,
    input: FrameInput,
    edit_seen: bool,
}

impl ValueSliderState {
    pub fn is_editing(&self) -> bool {
        self.edit.is_some()
    }

    pub fn begin_frame(&mut self, input: &FrameInput) {
        // Discard an edit when its document, layer, tool or panel disappeared.
        if !self.edit_seen {
            self.edit = None;
        }
        self.edit_seen = false;
        if !input.mouse_down {
            self.drag = None;
        }
        self.input = input.clone();
    }

    pub fn draw(
        &mut self,
        ui: &mut Ui,
        id: &str,
        value: &mut f32,
        slider: ValueSlider<'_>,
    ) -> Response {
        let before = *value;
        let scale = ui.scale();
        let area = ui.area(
            id,
            Vec2::new(ui.available_size().x.max(40.0 * scale), 28.0 * scale),
        );
        let pointer = ui.pointer();
        let enabled = ui.enabled();
        let rect = area.rect;
        let arrow_x = (rect.max.x - 20.0 * scale).max(rect.min.x);
        let body = Rect {
            max: Vec2::new(arrow_x, rect.max.y),
            ..rect
        };
        let midpoint = (rect.min.y + rect.max.y) * 0.5;
        let own_edit = self.edit.as_ref().is_some_and(|edit| edit.id == id);
        if own_edit && !enabled {
            self.edit = None;
        }
        let mut edit_text = None;
        let mut invalid = false;
        if own_edit && enabled {
            self.edit_seen = true;
            let edit = self.edit.as_mut().unwrap();
            edit.input(&self.input);
            let parsed = slider.parse(&edit.text);
            invalid = parsed.is_none();
            let outside = pointer.pressed && !rect.contains(pointer.pos);
            if self.input.key_escape {
                self.edit = None;
            } else if (self.input.key_enter && parsed.is_some()) || outside {
                if let Some(parsed) = parsed {
                    *value = parsed;
                }
                self.edit = None;
            } else {
                edit_text = Some((edit.text.clone(), edit.caret, edit.selection()));
            }
        } else if enabled {
            if area.hovered && pointer.right_pressed && body.contains(pointer.pos) {
                let text = slider.format(*value);
                self.edit = Some(Edit {
                    id: id.into(),
                    caret: text.len(),
                    anchor: 0,
                    text,
                });
                self.edit_seen = true;
                self.drag = None;
                ui.request_repaint();
            } else if area.hovered && pointer.pressed {
                if pointer.pos.x >= arrow_x {
                    let direction = if pointer.pos.y < midpoint { 1.0 } else { -1.0 };
                    *value = (*value + direction * slider.step).clamp(slider.min, slider.max);
                    self.drag = None;
                } else {
                    self.drag = Some(id.into());
                }
            }
            if self.drag.as_deref() == Some(id) && area.active && pointer.down {
                let t = ((pointer.pos.x - body.min.x) / body.width().max(1.0)).clamp(0.0, 1.0);
                let raw = slider.min + t * (slider.max - slider.min);
                *value = (slider.min + ((raw - slider.min) / slider.step).round() * slider.step)
                    .clamp(slider.min, slider.max);
            }
        }

        let theme = *ui.theme();
        let editing = self.edit.as_ref().is_some_and(|edit| edit.id == id) && enabled;
        let accent = [0.13, 0.23, 0.36, 1.0];
        let divider = [0.045, 0.055, 0.075, 1.0];
        let border = if invalid {
            [0.8, 0.25, 0.25, 1.0]
        } else if editing || (area.hovered && enabled) {
            accent
        } else {
            theme.input.border
        };
        ui.fill_round(rect, 3.0 * scale, border);
        ui.fill_round(
            rect.inset(scale),
            2.0 * scale,
            if editing {
                [0.025, 0.035, 0.05, 1.0]
            } else if enabled {
                theme.input.bg
            } else {
                theme.input.bg_disabled
            },
        );
        let inner = body.inset(2.0 * scale);
        if let Some((text, caret, selection)) = edit_text {
            // Numeric input uses fixed character cells, so selection and caret
            // are actual geometry rather than a font-dependent caret glyph.
            let advance = 8.0 * scale;
            let origin = rect.min + Vec2::new(8.0, 5.0) * scale;
            let capacity = ((body.width() - 18.0 * scale) / advance).floor().max(1.0) as usize;
            let start = caret.saturating_sub(capacity);
            let end = (start + capacity).min(text.len());
            let selected_start = selection.start.max(start);
            let selected_end = selection.end.min(end);
            if selected_start < selected_end {
                ui.fill_round(
                    Rect::from_min_size(
                        Vec2::new(
                            origin.x + (selected_start - start) as f32 * advance - scale,
                            rect.min.y + 4.0 * scale,
                        ),
                        Vec2::new(
                            (selected_end - selected_start) as f32 * advance + 2.0 * scale,
                            20.0 * scale,
                        ),
                    ),
                    2.0 * scale,
                    [0.0865, 0.1683, 0.2705, 1.0],
                );
            }
            for (index, character) in text[start..end].chars().enumerate() {
                ui.text_at(
                    origin + Vec2::new(index as f32 * advance, 0.0),
                    &character.to_string(),
                );
            }
            let caret_x = origin.x + (caret - start) as f32 * advance;
            ui.line(
                Vec2::new(caret_x, rect.min.y + 6.0 * scale),
                Vec2::new(caret_x, rect.max.y - 6.0 * scale),
                scale,
                accent,
            );
        } else {
            let fraction = ((*value - slider.min) / (slider.max - slider.min)).clamp(0.0, 1.0);
            if fraction > 0.0 {
                ui.fill_round(
                    Rect {
                        max: Vec2::new(inner.min.x + inner.width() * fraction, inner.max.y),
                        ..inner
                    },
                    2.0 * scale,
                    if enabled {
                        if self.drag.as_deref() == Some(id) {
                            [0.11, 0.21, 0.33, 1.0]
                        } else if area.hovered {
                            [0.10, 0.19, 0.30, 1.0]
                        } else {
                            // Krita reference: sRGB #53728E, converted to linear RGB.
                            [0.0865, 0.1683, 0.2705, 1.0]
                        }
                    } else {
                        theme.slider.track
                    },
                );
            }
            let space = if slider.suffix == "%" { "" } else { " " };
            ui.text_at(
                rect.min + Vec2::new(8.0, 5.0) * scale,
                &format!(
                    "{}: {}{space}{}",
                    slider.label,
                    slider.format(*value),
                    slider.suffix
                ),
            );
        }
        let arrows = Rect {
            min: Vec2::new(arrow_x + scale, rect.min.y + scale),
            max: rect.max - Vec2::splat(scale),
        };
        ui.fill_round(
            arrows,
            scale,
            if enabled {
                [0.035, 0.045, 0.06, 1.0]
            } else {
                theme.input.bg_disabled
            },
        );
        if enabled && area.hovered && pointer.pos.x >= arrow_x {
            let hover = if pointer.pos.y < midpoint {
                Rect {
                    max: Vec2::new(arrows.max.x, midpoint),
                    ..arrows
                }
            } else {
                Rect {
                    min: Vec2::new(arrows.min.x, midpoint),
                    ..arrows
                }
            };
            ui.fill_rect(hover, [0.055, 0.08, 0.12, 1.0]);
        }
        let color = if enabled {
            theme.text.dim
        } else {
            theme.text.disabled
        };
        ui.line(
            Vec2::new(arrow_x, rect.min.y + scale),
            Vec2::new(arrow_x, rect.max.y - scale),
            scale,
            divider,
        );
        ui.line(
            Vec2::new(arrow_x, midpoint),
            Vec2::new(rect.max.x, midpoint),
            scale,
            divider,
        );
        for (y, direction) in [
            (rect.min.y + 7.0 * scale, -1.0),
            (rect.max.y - 7.0 * scale, 1.0),
        ] {
            let x = (arrow_x + rect.max.x) * 0.5;
            let tip = Vec2::new(x, y + direction * 2.0 * scale);
            ui.line(
                Vec2::new(x - 3.0 * scale, y - direction * scale),
                tip,
                scale,
                color,
            );
            ui.line(
                tip,
                Vec2::new(x + 3.0 * scale, y - direction * scale),
                scale,
                color,
            );
        }
        if area.hovered && enabled {
            ui.set_mouse_cursor(if pointer.pos.x >= arrow_x {
                CursorIcon::Pointer
            } else if editing {
                CursorIcon::Text
            } else {
                CursorIcon::ResizeEw
            });
        }
        let changed = *value != before;
        if changed {
            ui.request_repaint();
        }
        Response {
            hovered: area.hovered,
            clicked: area.active && pointer.released,
            changed,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(
        ui: &mut Ui,
        state: &mut ValueSliderState,
        value: &mut f32,
        input: &FrameInput,
        percent: bool,
        enabled: bool,
    ) -> Response {
        state.begin_frame(input);
        ui.begin_frame(mega_ui::UiInput {
            viewport: Vec2::new(200.0, 200.0),
            mouse_pos: input.mouse_pos,
            mouse_pressed: input.mouse_pressed,
            mouse_down: input.mouse_down,
            mouse_released: input.mouse_released,
            mouse_right_pressed: input.mouse_right_pressed,
            mouse_right_down: input.mouse_right_down,
            ..Default::default()
        });
        let mut response = Response::default();
        ui.add_enabled(enabled, |ui| {
            response = state.draw(
                ui,
                "test",
                value,
                if percent {
                    ValueSlider::percent("Opacity", 0.0)
                } else {
                    ValueSlider::new("Size", 1.0, 256.0, 1.0, "px")
                },
            );
        });
        ui.end_frame();
        response
    }

    fn open(ui: &mut Ui, state: &mut ValueSliderState, value: &mut f32) {
        let mut input = FrameInput::default();
        input.mouse_pos = Vec2::new(50.0, 14.0);
        input.mouse_right_pressed = true;
        input.mouse_right_down = true;
        frame(ui, state, value, &input, true, true);
        assert!(state.is_editing());
    }

    #[test]
    fn drag_clamps_steps_and_arrow_press_is_not_repeated_on_settling_frames() {
        let mut ui = Ui::new();
        let mut state = ValueSliderState::default();
        let mut value = 24.0;
        let mut input = FrameInput::default();
        input.mouse_pos = Vec2::new(90.0, 14.0);
        input.mouse_pressed = true;
        input.mouse_down = true;
        assert!(frame(&mut ui, &mut state, &mut value, &input, false, true).changed());
        assert_eq!(value, 129.0);
        input.mouse_pressed = false;
        input.mouse_pos.x = 400.0;
        frame(&mut ui, &mut state, &mut value, &input, false, true);
        assert_eq!(value, 256.0);
        input.mouse_pos.x = -50.0;
        frame(&mut ui, &mut state, &mut value, &input, false, true);
        assert_eq!(value, 1.0);
        input.mouse_down = false;
        input.mouse_released = true;
        frame(&mut ui, &mut state, &mut value, &input, false, true);
        input.mouse_released = false;
        input.mouse_pressed = true;
        input.mouse_down = true;
        input.mouse_pos = Vec2::new(190.0, 5.0);
        frame(&mut ui, &mut state, &mut value, &input, false, true);
        assert_eq!(value, 2.0);
        input.mouse_pressed = false;
        for _ in 0..3 {
            frame(&mut ui, &mut state, &mut value, &input, false, true);
        }
        assert_eq!(value, 2.0);
    }

    #[test]
    fn percent_edit_commits_exact_value_enter_and_cancels_escape() {
        let mut ui = Ui::new();
        let mut state = ValueSliderState::default();
        let mut value = 0.5;
        open(&mut ui, &mut state, &mut value);
        let mut input = FrameInput::default();
        input.text = "12,5".into();
        frame(&mut ui, &mut state, &mut value, &input, true, true);
        assert_eq!(value, 0.5);
        input.text.clear();
        input.key_enter = true;
        assert!(frame(&mut ui, &mut state, &mut value, &input, true, true).changed());
        assert_eq!(value, 0.125);
        assert!(!state.is_editing());
        open(&mut ui, &mut state, &mut value);
        input = FrameInput::default();
        input.text = "90".into();
        frame(&mut ui, &mut state, &mut value, &input, true, true);
        input.text.clear();
        input.key_escape = true;
        frame(&mut ui, &mut state, &mut value, &input, true, true);
        assert_eq!(value, 0.125);
        assert!(!state.is_editing());
    }

    #[test]
    fn invalid_edit_stays_open_and_disabled_widgets_never_change_values() {
        let mut ui = Ui::new();
        let mut state = ValueSliderState::default();
        let mut value = 0.5;
        open(&mut ui, &mut state, &mut value);
        let mut input = FrameInput::default();
        input.text = "1e".into();
        frame(&mut ui, &mut state, &mut value, &input, true, true);
        input.text.clear();
        input.key_enter = true;
        frame(&mut ui, &mut state, &mut value, &input, true, true);
        assert!(state.is_editing());
        assert_eq!(value, 0.5);
        input.key_enter = false;
        input.key_select_all = true;
        input.text = "500".into();
        frame(&mut ui, &mut state, &mut value, &input, true, true);
        input = FrameInput::default();
        input.key_enter = true;
        frame(&mut ui, &mut state, &mut value, &input, true, true);
        assert_eq!(value, 1.0);
        input = FrameInput::default();
        input.mouse_pos = Vec2::new(30.0, 14.0);
        input.mouse_pressed = true;
        input.mouse_down = true;
        frame(&mut ui, &mut state, &mut value, &input, true, false);
        assert_eq!(value, 1.0);
        input.mouse_pressed = false;
        input.mouse_right_pressed = true;
        frame(&mut ui, &mut state, &mut value, &input, true, false);
        assert!(!state.is_editing());
    }

    #[test]
    fn switching_widget_does_not_apply_an_old_edit_to_its_replacement() {
        let mut ui = Ui::new();
        let mut state = ValueSliderState::default();
        let mut value = 0.5;
        open(&mut ui, &mut state, &mut value);
        state.begin_frame(&FrameInput::default()); // different panel: no draw for this field
        state.begin_frame(&FrameInput::default());
        assert!(!state.is_editing());
        assert_eq!(value, 0.5);
    }
    #[test]
    fn editor_routes_numeric_keys_to_the_component_without_canvas_shortcuts() {
        use crate::{app::EditorApp, host::Scene};
        let mut app = EditorApp::default();
        let mut ui = Ui::new();
        let mut raw = FrameInput::default();
        raw.mouse_pos = Vec2::new(50.0, 14.0);
        raw.mouse_right_pressed = true;
        raw.mouse_right_down = true;
        app.prepare_ui_input(&raw);
        app.begin_frame(&raw);
        ui.begin_frame(mega_ui::UiInput {
            viewport: Vec2::splat(200.0),
            mouse_pos: raw.mouse_pos,
            mouse_right_pressed: true,
            mouse_right_down: true,
            ..Default::default()
        });
        app.value_sliders.draw(
            &mut ui,
            "brush_size",
            &mut app.session.brush_size,
            ValueSlider::new("Size", 1.0, 256.0, 1.0, "px"),
        );
        ui.end_frame();
        assert!(app.wants_keyboard());
        raw = FrameInput::default();
        raw.text = "42".into();
        raw.key_enter = true;
        raw.key_undo = true;
        raw.key_delete = true;
        raw.key_tool_fill = true;
        app.prepare_ui_input(&raw);
        app.begin_frame(&raw);
        assert!(
            !app.input.key_enter
                && !app.input.key_undo
                && !app.input.key_delete
                && !app.input.key_tool_fill
        );
        ui.begin_frame(mega_ui::UiInput {
            viewport: Vec2::splat(200.0),
            ..Default::default()
        });
        app.value_sliders.draw(
            &mut ui,
            "brush_size",
            &mut app.session.brush_size,
            ValueSlider::new("Size", 1.0, 256.0, 1.0, "px"),
        );
        ui.end_frame();
        assert_eq!(app.session.brush_size, 42.0);
        assert!(!app.wants_keyboard());
    }
}
