use egui::Context;
use egui_material_icons::icons;

use crate::debugger::EmulatorState;

pub fn show_controls(ctx: &Context, open: &mut bool, state: &mut EmulatorState) {
    egui::Window::new("Controls")
        .open(open)
        .resizable(false)
        .default_size(egui::vec2(160.0, 0.0))
        .show(ctx, |ui| {
            let is_paused = *state == EmulatorState::Paused;
            let is_running = *state == EmulatorState::Running;

            ui.set_min_width(140.0);

            let btn_size = egui::vec2(ui.available_width(), 0.0);

            if ui
                .add_enabled(
                    is_paused,
                    egui::Button::new(format!("{} Continue", icons::ICON_PLAY_ARROW)).min_size(btn_size),
                )
                .clicked()
            {
                *state = EmulatorState::Running;
            }

            if ui
                .add_enabled(
                    is_running,
                    egui::Button::new(format!("{} Pause", icons::ICON_PAUSE)).min_size(btn_size),
                )
                .clicked()
            {
                *state = EmulatorState::Paused;
            }

            if ui
                .add_enabled(
                    is_paused,
                    egui::Button::new(format!("{} Step", icons::ICON_SKIP_NEXT)).min_size(btn_size),
                )
                .clicked()
            {
                *state = EmulatorState::Step;
            }

            if ui
                .add(egui::Button::new(format!("{} Run Until VSync", icons::ICON_FAST_FORWARD)).min_size(btn_size))
                .clicked()
            {
                *state = EmulatorState::RunUntilVsync;
            }

            ui.separator();

            ui.horizontal(|ui| {
                let mut run_until_addr_input = "";
                let resp = ui.add_enabled(
                    is_paused,
                    egui::TextEdit::singleline(&mut run_until_addr_input)
                        .hint_text("address")
                        .desired_width(80.0)
                        .font(egui::TextStyle::Monospace),
                );
                let go = ui
                    .add_enabled(is_paused, egui::Button::new(format!("{} Run", icons::ICON_PLAY_ARROW)))
                    .clicked();
                if go || (resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter))) {
                    let s = run_until_addr_input.trim().trim_start_matches("0x");
                    if let Ok(addr) = u32::from_str_radix(s, 16) {
                        *state = EmulatorState::RunUntilAddress(addr);
                    }
                }
            });
        });
}
