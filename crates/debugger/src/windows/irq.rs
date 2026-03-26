use egui::{Align, Color32, Context, Grid, Layout, RichText};
use gecko::cpu::Cpu;
use gecko::flipper::pi::ProcessorInterface;

fn centered_icon(ui: &mut egui::Ui, icon: &str, color: Color32) {
    ui.with_layout(Layout::top_down(Align::Center), |ui| {
        ui.label(RichText::new(icon).color(color));
    });
}

fn interrupt_row(ui: &mut egui::Ui, name: &str, pending: bool, masked: bool) {
    let name_color = if pending && masked {
        Color32::from_rgb(255, 180, 60)
    } else {
        Color32::PLACEHOLDER
    };

    let (pending_icon, pending_color) = if pending {
        (egui_phosphor::regular::CHECK_CIRCLE, Color32::from_rgb(255, 180, 60))
    } else {
        (egui_phosphor::regular::CIRCLE, Color32::from_rgb(70, 70, 70))
    };

    let (mask_icon, mask_color) = if masked {
        (egui_phosphor::regular::CHECK_SQUARE, Color32::from_rgb(100, 180, 255))
    } else {
        (egui_phosphor::regular::SQUARE, Color32::from_rgb(70, 70, 70))
    };

    ui.label(RichText::new(name).color(name_color));
    centered_icon(ui, pending_icon, pending_color);
    ui.label(RichText::new(mask_icon).color(mask_color));
    ui.end_row();
}

pub fn show_irq(ctx: &Context, open: &mut bool, cpu: &Cpu, pi: &ProcessorInterface) {
    egui::Window::new("IRQ")
        .open(open)
        .default_size(egui::vec2(380.0, 480.0))
        .show(ctx, |ui| {
            let msr = &cpu.msr;
            let spr = &cpu.spr;
            let ee = msr.external_interrupt_enable();

            // MSR.EE, SRR0/SRR1
            Grid::new("top_state").num_columns(2).striped(true).show(ui, |ui| {
                ui.label("MSR.EE");
                ui.horizontal(|ui| {
                    let (icon, color) = if ee {
                        (egui_phosphor::regular::CHECK_CIRCLE, Color32::from_rgb(80, 220, 80))
                    } else {
                        (egui_phosphor::regular::X_CIRCLE, Color32::from_rgb(200, 60, 60))
                    };
                    ui.label(RichText::new(icon).color(color));
                    ui.label(if ee {
                        "interrupts enabled"
                    } else {
                        "interrupts disabled"
                    });
                });
                ui.end_row();

                ui.label("SRR0");
                ui.monospace(format!("{:#010X}", spr.srr0.raw()));
                ui.end_row();

                ui.label("SRR1");
                ui.monospace(format!("{:#010X}", spr.srr1));
                ui.end_row();
            });

            ui.add_space(8.0);

            // PI Table
            ui.strong("PI Interrupts");
            ui.separator();

            Grid::new("pi_ints")
                .num_columns(3)
                .striped(true)
                .min_col_width(0.0)
                .show(ui, |ui| {
                    ui.label(RichText::new("Source").strong());
                    ui.label(RichText::new("Pending").strong());
                    ui.label(RichText::new("Masked").strong());
                    ui.end_row();

                    let sr = &pi.intsr;
                    let mr = &pi.intmr;

                    interrupt_row(ui, "GP Error", sr.gp_runtime_error(), mr.gp_runtime_error());
                    interrupt_row(ui, "Reset Switch", sr.reset_switch(), mr.reset_switch());
                    interrupt_row(ui, "DVD", sr.dvd(), mr.dvd());
                    interrupt_row(ui, "Serial", sr.serial(), mr.serial());
                    interrupt_row(ui, "EXI", sr.exi(), mr.exi());
                    interrupt_row(ui, "Streaming", sr.streaming(), mr.streaming());
                    interrupt_row(ui, "DSP", sr.dsp(), mr.dsp());
                    interrupt_row(ui, "Memory", sr.memory(), mr.memory());
                    interrupt_row(ui, "Video", sr.video(), mr.video());
                    interrupt_row(
                        ui,
                        "PE Token",
                        sr.token_assertion_in_cmd_list(),
                        mr.token_assertion_in_cmd_list(),
                    );
                    interrupt_row(ui, "PE Finish", sr.frame_is_ready(), mr.frame_is_ready());
                    interrupt_row(ui, "Cmd FIFO", sr.command_fifo(), mr.command_fifo());
                    interrupt_row(ui, "Debug", sr.debug(), mr.debug());
                    interrupt_row(ui, "Hi-Speed Port", sr.highspeed_port(), mr.highspeed_port());
                });

            ui.horizontal(|ui| {
                ui.label("INTSR");
                ui.monospace(format!("{:#010X}", pi.intsr.raw()));
                ui.add_space(12.0);
                ui.label("INTMR");
                ui.monospace(format!("{:#010X}", pi.intmr.raw()));
            });
        });
}
