mod bluetooth;
mod crypto;
mod l2cap;
mod wiimote;

use crate::hollywood::ipc::{DeviceContext, IosDevice};
pub(crate) use bluetooth::{WIIMOTE_ADDR, WIIMOTE_NAME};
use std::collections::VecDeque;
pub use wiimote::{
    BTN_A, BTN_B, BTN_DOWN, BTN_HOME, BTN_LEFT, BTN_MINUS, BTN_ONE, BTN_PLUS, BTN_RIGHT, BTN_TWO, BTN_UP,
    IR_CAMERA_HEIGHT, IR_CAMERA_WIDTH, NUNCHUK_BTN_C, NUNCHUK_BTN_Z, NUNCHUK_STICK_CENTER, NUNCHUK_STICK_MAX,
    NUNCHUK_STICK_MIN,
};

const USB_CTRL: u32 = 0;
const USB_BULK: u32 = 1;
const USB_INTR: u32 = 2;
const EP_INT_IN: u8 = 0x81;
const EP_BULK_OUT: u8 = 0x02;
const EP_BULK_IN: u8 = 0x82;

const MAX_PENDING_QUEUE: usize = 256;

pub struct Bluetooth {
    pub(self) pending_hci: VecDeque<Vec<u8>>,
    pub(self) pending_acl: VecDeque<Vec<u8>>,
    pub(self) wiimote: wiimote::WiimoteState,
    pub(self) scanning: bool,
    pub(self) connection_requested: bool,
    pub(self) connected: bool,
    pub(self) host_hid_control_cid: Option<u16>,
    pub(self) host_hid_interrupt_cid: Option<u16>,
    pub(self) next_l2cap_ident: u8,
}

impl Bluetooth {
    pub fn new() -> Self {
        Self {
            pending_hci: VecDeque::new(),
            pending_acl: VecDeque::new(),
            wiimote: wiimote::WiimoteState::default(),
            scanning: false,
            connection_requested: false,
            connected: false,
            host_hid_control_cid: None,
            host_hid_interrupt_cid: None,
            next_l2cap_ident: 2,
        }
    }
}

impl IosDevice for Bluetooth {
    fn ioctlv(&mut self, ctx: &mut DeviceContext<'_>, cmd: u32, _in_count: u32, _io_count: u32, vec_ptr: u32) -> i32 {
        match cmd {
            USB_CTRL => self.handle_control(ctx, vec_ptr),
            USB_INTR => self.handle_interrupt(ctx, vec_ptr),
            USB_BULK => self.handle_bulk(ctx, vec_ptr),
            _ => 0,
        }
    }

    fn set_wiimote_buttons(&mut self, buttons: u16) -> bool {
        Bluetooth::set_wiimote_buttons(self, buttons)
    }

    fn set_wiimote_shake(&mut self, active: bool) {
        Bluetooth::set_wiimote_shake(self, active)
    }

    fn set_nunchuk(&mut self, buttons: u8, stick_x: u8, stick_y: u8) -> bool {
        Bluetooth::set_nunchuk(self, buttons, stick_x, stick_y)
    }

    fn set_ir_pointer(&mut self, pointer: Option<(u16, u16)>) -> bool {
        Bluetooth::set_ir_pointer(self, pointer)
    }
}

impl Bluetooth {
    fn handle_control(&mut self, ctx: &mut DeviceContext<'_>, vec_ptr: u32) -> i32 {
        let bm_request = ctx.mmio.phys_read_u8(self::vec_data(ctx.mmio, vec_ptr, 0));
        let b_request = ctx.mmio.phys_read_u8(self::vec_data(ctx.mmio, vec_ptr, 1));
        if bm_request != 0x20 || b_request != 0 {
            return 0;
        }

        let w_length = {
            let p = self::vec_data(ctx.mmio, vec_ptr, 4);
            u16::from_le_bytes([ctx.mmio.phys_read_u8(p), ctx.mmio.phys_read_u8(p + 1)])
        };
        if w_length < 3 {
            return 0;
        }

        let data_ptr = self::vec_data(ctx.mmio, vec_ptr, 6);
        let packet = ctx.mmio.phys_slice(data_ptr, w_length as usize).to_vec();
        if packet.len() < 2 {
            return 0;
        }

        self.handle_hci_command(&packet);
        0
    }

    fn handle_bulk(&mut self, ctx: &mut DeviceContext<'_>, vec_ptr: u32) -> i32 {
        let endpoint = ctx.mmio.phys_read_u8(self::vec_data(ctx.mmio, vec_ptr, 0));
        let len_p = self::vec_data(ctx.mmio, vec_ptr, 1);
        let w_length = u16::from_le_bytes([ctx.mmio.phys_read_u8(len_p), ctx.mmio.phys_read_u8(len_p + 1)]);
        let buf_ptr = self::vec_data(ctx.mmio, vec_ptr, 2);

        match endpoint {
            EP_BULK_OUT => {
                let packet = ctx.mmio.phys_slice(buf_ptr, w_length as usize).to_vec();
                tracing::debug!(
                    bytes = format!("{packet:02X?}"),
                    "received Bluetooth ACL frame from guest"
                );
                self.handle_acl_out(&packet);
                // Without this the host's HCI tx-buffer fills up
                // after 8? ACL packets and it disconnects the wiimote.
                self.queue_number_of_completed_packets(1);
                w_length as i32
            }
            EP_BULK_IN => {
                let Some(packet) = self.pending_acl.pop_front() else {
                    return 0;
                };

                let delivered = packet.len().min(w_length as usize);
                ctx.mmio
                    .phys_slice_mut(buf_ptr, delivered)
                    .copy_from_slice(&packet[..delivered]);

                tracing::debug!(
                    len = delivered,
                    bytes = format!("{:02X?}", &packet[..delivered]),
                    "delivering Bluetooth ACL frame to guest"
                );

                delivered as i32
            }
            _ => 0,
        }
    }

    /// HCI event receive (interrupt IN endpoint 0x81). Deliver the next
    /// queued event, or reply length 0 (the SDK seems to tolerate that).
    fn handle_interrupt(&mut self, ctx: &mut DeviceContext<'_>, vec_ptr: u32) -> i32 {
        if ctx.mmio.phys_read_u8(self::vec_data(ctx.mmio, vec_ptr, 0)) != EP_INT_IN {
            return 0;
        }

        let Some(event) = self.pending_hci.pop_front() else {
            return 0;
        };

        let buf_ptr = self::vec_data(ctx.mmio, vec_ptr, 2);
        ctx.mmio.phys_slice_mut(buf_ptr, event.len()).copy_from_slice(&event);

        tracing::debug!(
            event = format!("{:02X?}", &event),
            "delivering Bluetooth HCI event to guest"
        );

        event.len() as i32
    }

    fn set_wiimote_buttons(&mut self, buttons: u16) -> bool {
        let changed = self.wiimote.set_buttons(buttons);

        if changed {
            tracing::debug!(buttons = format!("{buttons:#06x}"), "Wiimote button state changed");
        }

        if self.host_hid_interrupt_cid.is_none() {
            return changed;
        }

        let report = self.wiimote.make_input_report();
        self.queue_hid_input_report(report);

        changed
    }

    fn set_wiimote_shake(&mut self, active: bool) {
        let report_needed = self.wiimote.tick_shake(active);
        if !report_needed || self.host_hid_interrupt_cid.is_none() {
            return;
        }
        
        let report = self.wiimote.make_input_report();
        self.queue_hid_input_report(report);
    }

    fn set_nunchuk(&mut self, buttons: u8, stick_x: u8, stick_y: u8) -> bool {
        let changed = self.wiimote.set_nunchuk(buttons, stick_x, stick_y);

        if changed {
            tracing::debug!(
                buttons = format!("{buttons:#04x}"),
                stick_x = format!("{stick_x:#04x}"),
                stick_y = format!("{stick_y:#04x}"),
                "Nunchuk state changed"
            );
        }

        if self.host_hid_interrupt_cid.is_none() {
            return changed;
        }

        let report = self.wiimote.make_input_report();
        self.queue_hid_input_report(report);

        changed
    }

    fn set_ir_pointer(&mut self, pointer: Option<(u16, u16)>) -> bool {
        let changed = self.wiimote.set_ir_pointer(pointer);

        if changed {
            tracing::debug!(pointer = ?pointer, "IR pointer state changed");
        }

        if self.host_hid_interrupt_cid.is_none() {
            return changed;
        }

        let report = self.wiimote.make_input_report();
        self.queue_hid_input_report(report);

        changed
    }
}

#[inline(always)]
fn push_capped(queue: &mut VecDeque<Vec<u8>>, frame: Vec<u8>) {
    if queue.len() >= MAX_PENDING_QUEUE {
        tracing::warn!(
            cap = MAX_PENDING_QUEUE,
            "Bluetooth pending queue full; dropping oldest frame"
        );
        queue.pop_front();
    }

    queue.push_back(frame);
}

#[inline(always)]
fn vec_data<const SYS: crate::system::SystemId>(mmio: &crate::mmio::Mmio<SYS>, vec_ptr: u32, idx: u32) -> u32 {
    mmio.phys_read_u32(vec_ptr + (idx * 8))
}
