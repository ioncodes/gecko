use egui::{Context, Grid};
use gecko::dvd::DvdInterface;

use super::flag;

pub fn show_dvd(ctx: &Context, open: &mut bool, di: &DvdInterface) {
    egui::Window::new("DVD")
        .open(open)
        .default_size(egui::vec2(350.0, 300.0))
        .show(ctx, |ui| {
            // Disc info
            if let Some(dvd) = &di.dvd {
                let game_name = String::from_utf8_lossy(&dvd.header.game_name);
                let game_name = game_name.trim_end_matches('\0');
                let game_code = String::from_utf8_lossy(&dvd.header.game_code);
                let maker_code = String::from_utf8_lossy(&dvd.header.maker_code);

                ui.strong("Disc");
                Grid::new("dvd_disc_info").num_columns(2).striped(true).show(ui, |ui| {
                    ui.label("Game");
                    ui.monospace(game_name);
                    ui.end_row();

                    ui.label("ID");
                    ui.monospace(format!("{}{} v{}", game_code, maker_code, dvd.header.version));
                    ui.end_row();
                });
            } else {
                ui.label("No disc inserted");
            }

            ui.separator();

            // Status register
            ui.strong("Status (DISR)");
            Grid::new("dvd_status").num_columns(4).striped(true).show(ui, |ui| {
                ui.label("TCINT");
                flag(ui, di.status.transfer_complete());
                ui.label("Mask");
                flag(ui, di.status.transfer_complete_mask());
                ui.end_row();

                ui.label("DEINT");
                flag(ui, di.status.device_error());
                ui.label("Mask");
                flag(ui, di.status.device_error_mask());
                ui.end_row();

                ui.label("BRKINT");
                flag(ui, di.status.break_complete());
                ui.label("Mask");
                flag(ui, di.status.break_complete_mask());
                ui.end_row();

                ui.label("BRK");
                flag(ui, di.status.brk());
                ui.label("");
                ui.label("");
                ui.end_row();
            });

            ui.add_space(4.0);

            // Cover register
            ui.strong("Cover (DICVR)");
            Grid::new("dvd_cover").num_columns(4).striped(true).show(ui, |ui| {
                ui.label("Cover");
                flag(ui, di.cover.cover_status());
                ui.label("CVRINT");
                flag(ui, di.cover.cover_interrupt());
                ui.end_row();

                ui.label("Mask");
                flag(ui, di.cover.cover_interrupt_mask());
                ui.label("");
                ui.label("");
                ui.end_row();
            });

            ui.add_space(4.0);

            // Control register
            ui.strong("Control (DICR)");
            Grid::new("dvd_control").num_columns(4).striped(true).show(ui, |ui| {
                ui.label("TSTART");
                flag(ui, di.control.tstart());
                ui.label("DMA");
                ui.monospace(format!("{:?}", di.control.dma()));
                ui.end_row();

                ui.label("RW");
                ui.monospace(format!("{:?}", di.control.access_mode()));
                ui.label("");
                ui.label("");
                ui.end_row();
            });

            ui.add_space(4.0);

            // Command buffers & DMA
            ui.strong("Command / DMA");
            Grid::new("dvd_cmd_dma").num_columns(2).striped(true).show(ui, |ui| {
                ui.label("CMDBUF0");
                ui.monospace(format!("{:#010X}", di.cmdbuf0));
                ui.end_row();

                ui.label("CMDBUF1");
                ui.monospace(format!("{:#010X}", di.cmdbuf1));
                ui.end_row();

                ui.label("CMDBUF2");
                ui.monospace(format!("{:#010X}", di.cmdbuf2));
                ui.end_row();

                ui.label("IMMBUF");
                ui.monospace(format!("{:#010X}", di.immbuf));
                ui.end_row();

                ui.label("DMA Address");
                ui.monospace(format!("{:#010X}", di.dma_address.address()));
                ui.end_row();

                ui.label("DMA Length");
                ui.monospace(format!("{:#010X}", di.dma_length.length()));
                ui.end_row();
            });
        });
}
