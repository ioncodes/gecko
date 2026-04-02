use egui::Context;
use egui_phosphor::regular as icons;

use crate::EmulatorState;

pub fn show_controls(
    ctx: &Context,
    open: &mut bool,
    state: &mut EmulatorState,
    run_until_addr_input: &mut String,
    dvd_cover_open: &mut Option<bool>,
    tracing: bool,
    start_trace: &mut bool,
    stop_trace: &mut bool,
    dsp_tracing: bool,
    start_dsp_trace: &mut bool,
    stop_dsp_trace: &mut bool,
) {
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
                    egui::Button::new(format!("{} Continue", icons::PLAY)).min_size(btn_size),
                )
                .clicked()
            {
                *state = EmulatorState::Running;
            }

            if ui
                .add_enabled(
                    is_running,
                    egui::Button::new(format!("{} Pause", icons::PAUSE)).min_size(btn_size),
                )
                .clicked()
            {
                *state = EmulatorState::Paused;
            }

            if ui
                .add_enabled(
                    is_paused,
                    egui::Button::new(format!("{} Step CPU", icons::SKIP_FORWARD)).min_size(btn_size),
                )
                .clicked()
            {
                *state = EmulatorState::Step;
            }

            if ui
                .add_enabled(
                    is_paused,
                    egui::Button::new(format!("{} Step DSP", icons::SKIP_FORWARD)).min_size(btn_size),
                )
                .clicked()
            {
                *state = EmulatorState::StepDsp;
            }

            if ui
                .add(egui::Button::new(format!("{} Run Until VSync", icons::FAST_FORWARD)).min_size(btn_size))
                .clicked()
            {
                *state = EmulatorState::RunUntilVsync;
            }

            if ui
                .add_enabled(
                    is_paused,
                    egui::Button::new(format!("{} Run Until DSP", icons::FAST_FORWARD)).min_size(btn_size),
                )
                .clicked()
            {
                *state = EmulatorState::RunUntilDsp;
            }

            ui.separator();

            ui.horizontal(|ui| {
                egui::TextEdit::singleline(run_until_addr_input)
                    .hint_text("address")
                    .desired_width(80.0)
                    .font(egui::TextStyle::Monospace)
                    .show(ui);

                if ui.add(egui::Button::new(format!("{} Run", icons::PLAY))).clicked() {
                    let s = run_until_addr_input.trim().trim_start_matches("0x");
                    if let Ok(addr) = u32::from_str_radix(s, 16) {
                        *state = EmulatorState::RunUntilAddress(addr);
                    }
                }
            });

            ui.separator();

            if ui
                .add(egui::Button::new(format!("{} Open Cover", icons::EJECT)).min_size(btn_size))
                .clicked()
            {
                *dvd_cover_open = Some(true);
            }

            if ui
                .add(egui::Button::new(format!("{} Close Cover", icons::DISC)).min_size(btn_size))
                .clicked()
            {
                *dvd_cover_open = Some(false);
            }

            ui.separator();

            if !tracing {
                if ui
                    .add(egui::Button::new(format!("{} Start CPU Trace", icons::RECORD)).min_size(btn_size))
                    .clicked()
                {
                    *start_trace = true;
                }
            } else {
                if ui
                    .add(egui::Button::new(format!("{} Stop CPU Trace", icons::STOP)).min_size(btn_size))
                    .clicked()
                {
                    *stop_trace = true;
                }
            }

            if !dsp_tracing {
                if ui
                    .add(egui::Button::new(format!("{} Start DSP Trace", icons::RECORD)).min_size(btn_size))
                    .clicked()
                {
                    *start_dsp_trace = true;
                }
            } else {
                if ui
                    .add(egui::Button::new(format!("{} Stop DSP Trace", icons::STOP)).min_size(btn_size))
                    .clicked()
                {
                    *stop_dsp_trace = true;
                }
            }
        });
}
