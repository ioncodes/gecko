use egui::{Context, Grid};
use gecko::flipper::exi::ExternalInterface;

use super::flag;

fn chip_select_str(cs: u8) -> &'static str {
    match cs {
        0b001 => "slot 0",
        0b010 => "slot 1",
        0b100 => "slot 2",
        0b000 => "none",
        _ => "invalid",
    }
}

fn clock_mhz(clk: u8) -> &'static str {
    match clk {
        0 => "~1 MHz",
        1 => "~4 MHz",
        2 => "~8 MHz",
        3 => "~16 MHz",
        4 => "~32 MHz",
        _ => "?",
    }
}

struct ChannelInfo {
    name: &'static str,
    csr_raw: u32,
    exi_int_mask: bool,
    exi_int: bool,
    tc_int_mask: bool,
    tc_int: bool,
    ext_int_mask: bool,
    ext_int: bool,
    device_connected: bool,
    clock: u8,
    chip_select: u8,
    transfer_start: bool,
    dma_mode: bool,
    transfer_type: &'static str,
    transfer_length: u8,
    dma_addr: u32,
    dma_length: u32,
    data_raw: u32,
}

fn channel_info(exi: &ExternalInterface, ch: usize) -> ChannelInfo {
    use gecko::flipper::exi::regs::TransferType;
    match ch {
        0 => ChannelInfo {
            name: "CH0",
            csr_raw: exi.ch0_csr.raw(),
            exi_int_mask: exi.ch0_csr.exi_interrupt_mask(),
            exi_int: exi.ch0_csr.exi_interrupt(),
            tc_int_mask: exi.ch0_csr.tc_interrupt_mask(),
            tc_int: exi.ch0_csr.tc_interrupt(),
            ext_int_mask: exi.ch0_csr.ext_interrupt_mask(),
            ext_int: exi.ch0_csr.ext_interrupt(),
            device_connected: exi.ch0_csr.device_connected(),
            clock: exi.ch0_csr.clock(),
            chip_select: exi.ch0_csr.chip_select(),
            transfer_start: exi.ch0_cr.transfer_start(),
            dma_mode: exi.ch0_cr.dma_mode(),
            transfer_type: match exi.ch0_cr.transfer_type() {
                TransferType::Read => "read",
                TransferType::Write => "write",
                TransferType::ReadAndWrite => "read+write",
                TransferType::Reserved => "reserved",
            },
            transfer_length: exi.ch0_cr.transfer_length(),
            dma_addr: exi.ch0_mar.address() << 5,
            dma_length: exi.ch0_length.length() << 5,
            data_raw: exi.ch0_data.raw(),
        },
        1 => ChannelInfo {
            name: "CH1",
            csr_raw: exi.ch1_csr.raw(),
            exi_int_mask: exi.ch1_csr.exi_interrupt_mask(),
            exi_int: exi.ch1_csr.exi_interrupt(),
            tc_int_mask: exi.ch1_csr.tc_interrupt_mask(),
            tc_int: exi.ch1_csr.tc_interrupt(),
            ext_int_mask: exi.ch1_csr.ext_interrupt_mask(),
            ext_int: exi.ch1_csr.ext_interrupt(),
            device_connected: exi.ch1_csr.device_connected(),
            clock: exi.ch1_csr.clock(),
            chip_select: exi.ch1_csr.chip_select(),
            transfer_start: exi.ch1_cr.transfer_start(),
            dma_mode: exi.ch1_cr.dma_mode(),
            transfer_type: match exi.ch1_cr.transfer_type() {
                TransferType::Read => "read",
                TransferType::Write => "write",
                TransferType::ReadAndWrite => "read+write",
                TransferType::Reserved => "reserved",
            },
            transfer_length: exi.ch1_cr.transfer_length(),
            dma_addr: exi.ch1_mar.address() << 5,
            dma_length: exi.ch1_length.length() << 5,
            data_raw: exi.ch1_data.raw(),
        },
        2 => ChannelInfo {
            name: "CH2",
            csr_raw: exi.ch2_csr.raw(),
            exi_int_mask: exi.ch2_csr.exi_interrupt_mask(),
            exi_int: exi.ch2_csr.exi_interrupt(),
            tc_int_mask: exi.ch2_csr.tc_interrupt_mask(),
            tc_int: exi.ch2_csr.tc_interrupt(),
            ext_int_mask: exi.ch2_csr.ext_interrupt_mask(),
            ext_int: exi.ch2_csr.ext_interrupt(),
            device_connected: exi.ch2_csr.device_connected(),
            clock: exi.ch2_csr.clock(),
            chip_select: exi.ch2_csr.chip_select(),
            transfer_start: exi.ch2_cr.transfer_start(),
            dma_mode: exi.ch2_cr.dma_mode(),
            transfer_type: match exi.ch2_cr.transfer_type() {
                TransferType::Read => "read",
                TransferType::Write => "write",
                TransferType::ReadAndWrite => "read+write",
                TransferType::Reserved => "reserved",
            },
            transfer_length: exi.ch2_cr.transfer_length(),
            dma_addr: exi.ch2_mar.address() << 5,
            dma_length: exi.ch2_length.length() << 5,
            data_raw: exi.ch2_data.raw(),
        },
        _ => unreachable!(),
    }
}

fn show_channel(ui: &mut egui::Ui, ch: &ChannelInfo) {
    ui.push_id(ch.name, |ui| {
        ui.strong(ch.name);
        ui.separator();

        Grid::new(format!("{}_csr", ch.name))
            .num_columns(4)
            .striped(true)
            .show(ui, |ui| {
                ui.label("CSR");
                ui.monospace(format!("{:#010X}", ch.csr_raw));
                ui.label("");
                ui.label("");
                ui.end_row();

                ui.label("Device");
                flag(ui, ch.device_connected);
                ui.label("Clock");
                ui.monospace(format!("{} ({})", ch.clock, clock_mhz(ch.clock)));
                ui.end_row();

                ui.label("CS");
                ui.monospace(chip_select_str(ch.chip_select));
                ui.label("");
                ui.label("");
                ui.end_row();

                // Interrupt status row
                ui.label("EXIINT");
                ui.horizontal(|ui| {
                    flag(ui, ch.exi_int);
                    ui.label("Mask");
                    flag(ui, ch.exi_int_mask);
                });
                ui.label("TCINT");
                ui.horizontal(|ui| {
                    flag(ui, ch.tc_int);
                    ui.label("Mask");
                    flag(ui, ch.tc_int_mask);
                });
                ui.end_row();

                ui.label("EXTINT");
                ui.horizontal(|ui| {
                    flag(ui, ch.ext_int);
                    ui.label("Mask");
                    flag(ui, ch.ext_int_mask);
                });
                ui.label("");
                ui.label("");
                ui.end_row();
            });

        ui.add_space(4.0);

        Grid::new(format!("{}_cr", ch.name))
            .num_columns(4)
            .striped(true)
            .show(ui, |ui| {
                ui.label("CR");
                ui.label("");
                ui.label("");
                ui.label("");
                ui.end_row();

                ui.label("TSTART");
                flag(ui, ch.transfer_start);
                ui.label("DMA");
                flag(ui, ch.dma_mode);
                ui.end_row();

                ui.label("RW");
                ui.monospace(ch.transfer_type);
                ui.label("TLEN");
                ui.monospace(format!("{} B", ch.transfer_length + 1));
                ui.end_row();
            });

        ui.add_space(4.0);

        Grid::new(format!("{}_dma", ch.name))
            .num_columns(2)
            .striped(true)
            .show(ui, |ui| {
                ui.label("DMA");
                ui.label("");
                ui.end_row();

                ui.label("Addr");
                ui.monospace(format!("{:#010X}", ch.dma_addr));
                ui.end_row();

                ui.label("Length");
                ui.monospace(format!("{} B", ch.dma_length));
                ui.end_row();

                ui.label("Data");
                ui.monospace(format!("{:#010X}", ch.data_raw));
                ui.end_row();
            });
    });
}

pub fn show_exi(ctx: &Context, open: &mut bool, exi: &ExternalInterface) {
    egui::Window::new("EXI")
        .open(open)
        .default_size(egui::vec2(600.0, 400.0))
        .show(ctx, |ui| {
            ui.columns(3, |cols| {
                for (i, col) in cols.iter_mut().enumerate() {
                    let ch = channel_info(exi, i);
                    show_channel(col, &ch);
                }
            });
        });
}
