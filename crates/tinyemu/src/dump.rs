use colored::Colorize;
use gekko::exi::regs::TransferType;

use crate::snaptshot::CpuSnapshot;

pub fn registers(curr: &CpuSnapshot, prev: &CpuSnapshot) {
    let fmt_reg = |label: &str, val: u32, prev_val: u32| -> String {
        let value = format!("{:08X}", val);
        if val != prev_val {
            format!("{} {} ", label.yellow().bold(), value.bright_red().bold())
        } else {
            format!("{} {} ", label.dimmed(), value.dimmed())
        }
    };

    for row in 0..8 {
        let line: String = (0..4)
            .map(|col| {
                let i = row * 4 + col;
                fmt_reg(&format!("r{:<2}", i), curr.gprs[i], prev.gprs[i])
            })
            .collect();
        println!("{}", line.trim_end());
    }

    println!(
        "{}",
        format!(
            "{}{}",
            fmt_reg("lr ", curr.lr, prev.lr),
            fmt_reg("ctr", curr.ctr, prev.ctr)
        )
        .trim_end()
    );

    let cr_fields = [
        ("cr0", curr.cr.cr0(), prev.cr.cr0()),
        ("cr1", curr.cr.cr1(), prev.cr.cr1()),
        ("cr2", curr.cr.cr2(), prev.cr.cr2()),
        ("cr3", curr.cr.cr3(), prev.cr.cr3()),
        ("cr4", curr.cr.cr4(), prev.cr.cr4()),
        ("cr5", curr.cr.cr5(), prev.cr.cr5()),
        ("cr6", curr.cr.cr6(), prev.cr.cr6()),
        ("cr7", curr.cr.cr7(), prev.cr.cr7()),
    ];

    let fmt_cr_field =
        |label: &str, val: gekko::cpu::condition::ConditionField, prev_val: gekko::cpu::condition::ConditionField| {
            let flags = format!(
                "{}{}{}{}",
                if val.lt() { "L" } else { "·" },
                if val.gt() { "G" } else { "·" },
                if val.eq() { "Z" } else { "·" },
                if val.so() { "O" } else { "·" },
            );
            let text = format!("{}[{}] ", label, flags);
            if val.raw() != prev_val.raw() {
                format!("{}", text.bright_red().bold())
            } else {
                format!("{}", text.dimmed())
            }
        };

    let cr_line: String = cr_fields
        .iter()
        .map(|(label, val, prev_val)| fmt_cr_field(label, *val, *prev_val))
        .collect();
    println!("{}", cr_line.trim_end());

    println!();
}

pub fn memory(mmio: &gekko::mmio::Mmio, addr: u32) {
    let aligned_addr = addr & !0xF;
    let start = aligned_addr.wrapping_sub(0x40);
    let data = mmio.virt_slice(start, 0x80);

    for (i, line) in data.chunks(16).enumerate() {
        let line_addr = start.wrapping_add((i as u32) * 16);
        let hex = line
            .chunks(4)
            .map(|chunk| {
                let word = u32::from_be_bytes(chunk.try_into().unwrap());
                format!("{:08X}", word)
            })
            .collect::<Vec<_>>()
            .join(" ");

        println!("{} {}", format!("{:08X}:", line_addr).blue().bold(), hex);
    }
}

pub fn vi(vi: &gekko::vi::Vi) {
    println!("Display Configuration: {:?}", vi.dcr);
    println!("Bottom Field Base: {:08X?}", vi.bfbl);
    println!("Top Field Base: {:08X?}", vi.tfbl);
    println!("XFB Address: {:08X}", vi.xfb_addr());
}

fn fmt_selected_devices(cs: u8, device_names: &[&str; 3]) -> String {
    let mut selected = Vec::new();
    for i in 0..3u8 {
        if cs & (1 << i) != 0 {
            selected.push(format!("{} ({})", i, device_names[i as usize]));
        }
    }
    if selected.is_empty() {
        "none".to_string()
    } else {
        selected.join(", ")
    }
}

fn fmt_clock(clk: u8) -> &'static str {
    match clk {
        0 => "1 MHz",
        1 => "2 MHz",
        2 => "4 MHz",
        3 => "8 MHz",
        4 => "16 MHz",
        5 => "32 MHz",
        _ => "reserved",
    }
}

fn fmt_enabled(val: bool) -> &'static str {
    if val { "yes" } else { "no" }
}

fn fmt_pending(val: bool) -> &'static str {
    if val { "PENDING" } else { "clear" }
}

struct ExiChannelView {
    name: &'static str,
    device_names: &'static [&'static str; 3],
    csr_raw: u32,
    chip_select: u8,
    clock: u8,
    device_connected: bool,
    exi_interrupt: bool,
    exi_interrupt_mask: bool,
    tc_interrupt: bool,
    tc_interrupt_mask: bool,
    ext_interrupt: bool,
    ext_interrupt_mask: bool,
    rom_descramble_disabled: Option<bool>,
    dma_address: u32,
    dma_length: u32,
    transfer_start: bool,
    dma_mode: bool,
    transfer_type: TransferType,
    transfer_length: u8,
    immediate_data: u32,
}

fn exi_channel(ch: &ExiChannelView) {
    println!("  {} (CSR={:08X}):", ch.name, ch.csr_raw);
    println!(
        "    Selected Device:      {}",
        fmt_selected_devices(ch.chip_select, ch.device_names)
    );
    println!("    Clock:                {}", fmt_clock(ch.clock));
    println!("    Device Connected:     {}", fmt_enabled(ch.device_connected));
    if let Some(romdis) = ch.rom_descramble_disabled {
        println!("    ROM Descrambler Off:  {}", fmt_enabled(romdis));
    }
    println!(
        "    EXI Interrupt:        {} (enabled: {})",
        fmt_pending(ch.exi_interrupt),
        fmt_enabled(ch.exi_interrupt_mask)
    );
    println!(
        "    Transfer Complete:    {} (enabled: {})",
        fmt_pending(ch.tc_interrupt),
        fmt_enabled(ch.tc_interrupt_mask)
    );
    println!(
        "    External Interrupt:   {} (enabled: {})",
        fmt_pending(ch.ext_interrupt),
        fmt_enabled(ch.ext_interrupt_mask)
    );
    println!("    DMA Address:          0x{:08X}", ch.dma_address);
    println!("    DMA Length:           0x{:08X}", ch.dma_length);
    println!("    Transfer Started:     {}", fmt_enabled(ch.transfer_start));
    println!(
        "    Transfer Mode:        {}",
        if ch.dma_mode { "DMA" } else { "immediate" }
    );
    println!("    Transfer Direction:   {:?}", ch.transfer_type);
    println!("    Transfer Size:        {} byte(s)", ch.transfer_length as u32 + 1);
    println!("    Immediate Data:       0x{:08X}", ch.immediate_data);
}

pub fn exi(exi: &gekko::exi::Exi) {
    println!("EXI:");
    exi_channel(&ExiChannelView {
        name: "Channel 0",
        device_names: &["Memory Card Slot A", "Mask ROM/RTC/SRAM/UART", "Serial Port 1"],
        csr_raw: exi.ch0_csr.raw(),
        chip_select: exi.ch0_csr.chip_select(),
        clock: exi.ch0_csr.clock(),
        device_connected: exi.ch0_csr.device_connected(),
        exi_interrupt: exi.ch0_csr.exi_interrupt(),
        exi_interrupt_mask: exi.ch0_csr.exi_interrupt_mask(),
        tc_interrupt: exi.ch0_csr.tc_interrupt(),
        tc_interrupt_mask: exi.ch0_csr.tc_interrupt_mask(),
        ext_interrupt: exi.ch0_csr.ext_interrupt(),
        ext_interrupt_mask: exi.ch0_csr.ext_interrupt_mask(),
        rom_descramble_disabled: Some(exi.ch0_csr.rom_descramble_disabled()),
        dma_address: exi.ch0_mar.raw(),
        dma_length: exi.ch0_length.raw(),
        transfer_start: exi.ch0_cr.transfer_start(),
        dma_mode: exi.ch0_cr.dma_mode(),
        transfer_type: exi.ch0_cr.transfer_type(),
        transfer_length: exi.ch0_cr.transfer_length(),
        immediate_data: exi.ch0_data.raw(),
    });
    exi_channel(&ExiChannelView {
        name: "Channel 1",
        device_names: &["Memory Card Slot B", "-", "-"],
        csr_raw: exi.ch1_csr.raw(),
        chip_select: exi.ch1_csr.chip_select(),
        clock: exi.ch1_csr.clock(),
        device_connected: exi.ch1_csr.device_connected(),
        exi_interrupt: exi.ch1_csr.exi_interrupt(),
        exi_interrupt_mask: exi.ch1_csr.exi_interrupt_mask(),
        tc_interrupt: exi.ch1_csr.tc_interrupt(),
        tc_interrupt_mask: exi.ch1_csr.tc_interrupt_mask(),
        ext_interrupt: exi.ch1_csr.ext_interrupt(),
        ext_interrupt_mask: exi.ch1_csr.ext_interrupt_mask(),
        rom_descramble_disabled: None,
        dma_address: exi.ch1_mar.raw(),
        dma_length: exi.ch1_length.raw(),
        transfer_start: exi.ch1_cr.transfer_start(),
        dma_mode: exi.ch1_cr.dma_mode(),
        transfer_type: exi.ch1_cr.transfer_type(),
        transfer_length: exi.ch1_cr.transfer_length(),
        immediate_data: exi.ch1_data.raw(),
    });
    exi_channel(&ExiChannelView {
        name: "Channel 2",
        device_names: &["AD16 (debug)", "-", "-"],
        csr_raw: exi.ch2_csr.raw(),
        chip_select: exi.ch2_csr.chip_select(),
        clock: exi.ch2_csr.clock(),
        device_connected: exi.ch2_csr.device_connected(),
        exi_interrupt: exi.ch2_csr.exi_interrupt(),
        exi_interrupt_mask: exi.ch2_csr.exi_interrupt_mask(),
        tc_interrupt: exi.ch2_csr.tc_interrupt(),
        tc_interrupt_mask: exi.ch2_csr.tc_interrupt_mask(),
        ext_interrupt: exi.ch2_csr.ext_interrupt(),
        ext_interrupt_mask: exi.ch2_csr.ext_interrupt_mask(),
        rom_descramble_disabled: None,
        dma_address: exi.ch2_mar.raw(),
        dma_length: exi.ch2_length.raw(),
        transfer_start: exi.ch2_cr.transfer_start(),
        dma_mode: exi.ch2_cr.dma_mode(),
        transfer_type: exi.ch2_cr.transfer_type(),
        transfer_length: exi.ch2_cr.transfer_length(),
        immediate_data: exi.ch2_data.raw(),
    });
}
