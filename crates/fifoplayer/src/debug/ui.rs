use super::slice::CmdKind;
use super::{DebugSession, RunState, disasm};
use crate::playback::PlayerSink;
use backend_wgpu::GxRenderer;
use backend_wgpu::sink::InlineSink;
use egui::ViewportId;
use egui_phosphor::regular as icons;
use gecko::flipper::gx::constants::{BP_GEN_MODE, BP_PE_ALPHA_COMPARE, BP_PE_CMODE0, BP_PE_ZMODE};
use gecko::flipper::gx::regs::{AlphaCompare, BlendMode, GenMode, ZMode};
use gecko::system::SystemId;
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use winit::application::ApplicationHandler;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::window::{Window, WindowId};

#[derive(Copy, Clone, PartialEq, Eq)]
enum ViewMode {
    Auto,
    Efb,
    Xfb,
}

#[derive(Copy, Clone, PartialEq, Eq)]
enum Tab {
    Disasm,
    Registers,
    Hex,
    Breakpoints,
}

#[derive(Copy, Clone, PartialEq, Eq)]
enum Selection {
    None,
    Cmd(usize),
    Child(usize, usize),
}

#[derive(Clone)]
enum Row {
    Cmd(usize),
    DlChild { parent: usize, child: usize },
    Update(usize),
    FrameEnd,
}

enum UiAction {
    StepCmd,
    StepDraw,
    StepFrame,
    RunPause,
    RunToSelection,
    Restart,
    RestartFrame,
    JumpTo(usize, usize),
    ToggleDisabled(usize),
    ToggleRowBp(usize),
    ToggleExpand(usize),
    ApplyHex(usize, Vec<u8>),
    Revert(usize),
    Save,
}

pub fn run_debug<const SYSTEM: SystemId>(file: dff::DffFile, path: PathBuf, start: usize, end: usize) {
    let (instance, adapter, device, queue) = crate::init_wgpu();

    let (gx, sink) = InlineSink::new(device.clone(), queue.clone(), wgpu::TextureFormat::Rgba8Unorm);
    let sink = PlayerSink::new(Box::new(sink));
    let session = DebugSession::<SYSTEM>::new(file, path, start, end, sink);

    let egui_ctx = egui::Context::default();

    let mut fonts = egui::FontDefinitions::default();
    egui_phosphor::add_to_fonts(&mut fonts, egui_phosphor::Variant::Regular);
    if let Some(mono) = fonts.families.get_mut(&egui::FontFamily::Monospace) {
        mono.push("phosphor".into());
    }
    fonts
        .font_data
        .insert("phosphor-fill".into(), egui_phosphor::Variant::Fill.font_data().into());
    fonts.families.insert(
        egui::FontFamily::Name("phosphor-fill".into()),
        vec!["phosphor-fill".into()],
    );

    egui_ctx.set_fonts(fonts);

    let mut app = DebugApp::<SYSTEM> {
        session,
        gx,
        instance,
        adapter,
        device,
        queue,
        window: None,
        surface: None,
        egui_ctx,
        egui_winit: None,
        egui_renderer: None,
        efb_tex: None,
        xfb_tex: None,
        xfb_size: (1, 1),
        seen_presents: 0,
        view_mode: ViewMode::Auto,
        inspector_tab: Tab::Disasm,
        selection: Selection::None,
        expanded: HashSet::new(),
        rows: Vec::new(),
        cmd_flat: Vec::new(),
        rows_key: (usize::MAX, 0, 0),
        expanded_version: 0,
        scroll_to_current: false,
        hex_text: String::new(),
        hex_for: None,
        bp_input: String::new(),
        status_msg: String::new(),
    };

    let event_loop = EventLoop::new().unwrap();
    event_loop.run_app(&mut app).unwrap();
}

pub fn run_headless<const SYSTEM: SystemId>(
    file: dff::DffFile,
    path: PathBuf,
    start: usize,
    end: usize,
    out: &std::path::Path,
) {
    let (_instance, _adapter, device, queue) = crate::init_wgpu();

    let (gx, sink) = InlineSink::new(device.clone(), queue.clone(), wgpu::TextureFormat::Rgba8Unorm);
    let sink = PlayerSink::new(Box::new(sink));
    let mut session = DebugSession::<SYSTEM>::new(file, path, start, end, sink);

    session.resume(RunState::Running);

    while !session.finished {
        session.run_tick();
    }

    eprintln!("debug-stepped frames {start}..={end}, {} presents", session.presents);

    let _ = device.poll(wgpu::PollType::Wait {
        submission_index: None,
        timeout: None,
    });

    let g = gx.lock().unwrap();
    let captured = backend_wgpu::capture::capture_texture(&device, &queue, &g.xfb_texture).expect("XFB capture failed");
    backend_wgpu::capture::write_png(out, captured, true).expect("failed to write PNG");
    eprintln!("wrote {}", out.display());
}

struct DebugApp<const SYSTEM: SystemId> {
    session: DebugSession<SYSTEM>,
    gx: Arc<Mutex<GxRenderer>>,
    instance: wgpu::Instance,
    adapter: wgpu::Adapter,
    device: wgpu::Device,
    queue: wgpu::Queue,
    window: Option<Arc<Window>>,
    surface: Option<(wgpu::Surface<'static>, wgpu::SurfaceConfiguration)>,
    egui_ctx: egui::Context,
    egui_winit: Option<egui_winit::State>,
    egui_renderer: Option<egui_wgpu::Renderer>,
    efb_tex: Option<egui::TextureId>,
    xfb_tex: Option<egui::TextureId>,
    xfb_size: (u32, u32),
    seen_presents: u64,

    view_mode: ViewMode,
    inspector_tab: Tab,
    selection: Selection,
    expanded: HashSet<usize>,
    rows: Vec<Row>,
    cmd_flat: Vec<usize>,
    rows_key: (usize, u64, u64),
    expanded_version: u64,
    scroll_to_current: bool,
    hex_text: String,
    hex_for: Option<usize>,
    bp_input: String,
    status_msg: String,
}

const SURFACE_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Bgra8Unorm;

impl<const SYSTEM: SystemId> DebugApp<SYSTEM> {
    fn flush_gx(&self) {
        self.gx.lock().unwrap().debug_flush(&self.device, &self.queue);
    }

    fn refresh_xfb_binding(&mut self) {
        if self.seen_presents == self.session.presents {
            return;
        }
        self.seen_presents = self.session.presents;

        let (Some(renderer), Some(id)) = (self.egui_renderer.as_mut(), self.xfb_tex) else {
            return;
        };
        let g = self.gx.lock().unwrap();
        renderer.update_egui_texture_from_wgpu_texture(&self.device, &g.xfb_view, wgpu::FilterMode::Linear, id);

        let size = g.xfb_texture.size();
        self.xfb_size = (size.width, size.height);
    }

    fn rebuild_rows(&mut self) {
        let key = (
            self.session.frame_idx,
            self.session.index_version,
            self.expanded_version,
        );
        if self.rows_key == key {
            return;
        }
        self.rows_key = key;

        let cmds = &self.session.index.commands;
        let updates = &self.session.cur.updates;
        let mut rows = Vec::with_capacity(cmds.len() + updates.len() + 1);
        let mut cmd_flat = vec![0usize; cmds.len() + 1];
        let mut u = 0usize;

        for (i, cmd) in cmds.iter().enumerate() {
            while u < updates.len() && (updates[u].fifo_position as usize) < cmd.end() {
                rows.push(Row::Update(u));
                u += 1;
            }

            cmd_flat[i] = rows.len();
            rows.push(Row::Cmd(i));

            if let CmdKind::CallDl { children, .. } = &cmd.kind
                && self.expanded.contains(&cmd.offset)
            {
                for j in 0..children.len() {
                    rows.push(Row::DlChild { parent: i, child: j });
                }
            }
        }
        while u < updates.len() {
            rows.push(Row::Update(u));
            u += 1;
        }
        cmd_flat[cmds.len()] = rows.len();
        rows.push(Row::FrameEnd);

        self.rows = rows;
        self.cmd_flat = cmd_flat;
    }

    fn selected_command(&self) -> Option<&super::slice::Command> {
        match self.selection {
            Selection::None => None,
            Selection::Cmd(i) => self.session.index.commands.get(i),
            Selection::Child(p, c) => match &self.session.index.commands.get(p)?.kind {
                CmdKind::CallDl { children, .. } => children.get(c),
                _ => None,
            },
        }
    }

    fn apply_action(&mut self, action: UiAction) {
        let mut stepped = true;
        match action {
            UiAction::StepCmd => {
                self.session.step_command();
            }
            UiAction::StepDraw => self.session.step_draw(),
            UiAction::StepFrame => self.session.step_frame(),
            UiAction::RunPause => {
                stepped = false;

                if self.session.run_state == RunState::Paused {
                    if self.session.finished {
                        self.session.restart();
                        stepped = true;
                    }

                    self.session.resume(RunState::Running);
                } else {
                    self.session.pause();
                }
            }
            UiAction::RunToSelection => {
                if let Selection::Cmd(i) = self.selection {
                    let frame = self.session.frame_idx;

                    if i >= self.session.row {
                        self.session.resume(RunState::RunTo { frame, row: i });
                        stepped = false;
                    } else {
                        self.session.jump_to(frame, i);
                    }
                }
            }
            UiAction::Restart => self.session.restart(),
            UiAction::RestartFrame => self.session.restart_frame(),
            UiAction::JumpTo(frame, row) => self.session.jump_to(frame, row),
            UiAction::ToggleDisabled(off) => self.session.toggle_disabled(off),
            UiAction::ToggleRowBp(row) => {
                stepped = false;

                let key = (self.session.frame_idx, row);
                if !self.session.breakpoints.rows.remove(&key) {
                    self.session.breakpoints.rows.insert(key);
                }
            }
            UiAction::ToggleExpand(off) => {
                stepped = false;

                if !self.expanded.remove(&off) {
                    self.expanded.insert(off);
                }

                self.expanded_version += 1;
            }
            UiAction::ApplyHex(off, bytes) => {
                self.session.replace_command(off, bytes);

                self.hex_for = None;
                self.status_msg = "edit applied".into();
            }
            UiAction::Revert(off) => {
                self.session.revert_command(off);
                self.hex_for = None;
            }
            UiAction::Save => {
                stepped = false;

                match self.session.save_dff() {
                    Ok(path) => self.status_msg = format!("saved {}", path.display()),
                    Err(err) => self.status_msg = format!("save failed: {err:?}"),
                }
            }
        }

        if stepped {
            self.flush_gx();
            self.scroll_to_current = true;
        }
    }

    fn build_ui(&mut self, ui: &mut egui::Ui, actions: &mut Vec<UiAction>) {
        let ctx = ui.ctx().clone();

        if !ctx.egui_wants_keyboard_input() {
            ctx.input(|i| {
                if i.key_pressed(egui::Key::F5) {
                    actions.push(UiAction::RunPause);
                }
                if i.key_pressed(egui::Key::F10) {
                    actions.push(UiAction::StepCmd);
                }
                if i.key_pressed(egui::Key::F11) {
                    actions.push(UiAction::StepDraw);
                }
                if i.key_pressed(egui::Key::F8) {
                    actions.push(UiAction::StepFrame);
                }
            });
        }

        self.toolbar(ui, actions);
        self.frame_panel(ui, actions);
        self.command_panel(ui, actions);
        self.inspector_panel(ui, actions);
        self.game_panel(ui);
    }

    fn toolbar(&mut self, root: &mut egui::Ui, actions: &mut Vec<UiAction>) {
        let frame = egui::Frame::side_top_panel(root.style()).inner_margin(egui::Margin::symmetric(8, 6));

        egui::Panel::top("toolbar").frame(frame).show_inside(root, |ui| {
            ui.horizontal(|ui| {
                let running = self.session.run_state != RunState::Paused;

                if ui
                    .button(format!("{} Restart", icons::SKIP_BACK))
                    .on_hover_text("replay from the first frame")
                    .clicked()
                {
                    actions.push(UiAction::Restart);
                }

                if ui
                    .button(format!("{} Frame", icons::ARROW_COUNTER_CLOCKWISE))
                    .on_hover_text("restart the current frame")
                    .clicked()
                {
                    actions.push(UiAction::RestartFrame);
                }

                ui.separator();

                let run_label = if running {
                    format!("{} Pause", icons::PAUSE)
                } else {
                    format!("{} Run", icons::PLAY)
                };
                if ui.button(run_label).on_hover_text("F5").clicked() {
                    actions.push(UiAction::RunPause);
                }

                ui.add_enabled_ui(!running, |ui| {
                    if ui.button("Step").on_hover_text("F10: one command").clicked() {
                        actions.push(UiAction::StepCmd);
                    }

                    if ui.button("Draw").on_hover_text("F11: to the next draw").clicked() {
                        actions.push(UiAction::StepDraw);
                    }

                    if ui.button("Frame").on_hover_text("F8: to the next frame").clicked() {
                        actions.push(UiAction::StepFrame);
                    }

                    let has_sel = matches!(self.selection, Selection::Cmd(_));
                    if ui
                        .add_enabled(has_sel, egui::Button::new("To selection"))
                        .on_hover_text("run until the selected command")
                        .clicked()
                    {
                        actions.push(UiAction::RunToSelection);
                    }
                });

                ui.separator();

                egui::ComboBox::from_id_salt("view_mode")
                    .selected_text(match self.view_mode {
                        ViewMode::Auto => "Auto",
                        ViewMode::Efb => "EFB",
                        ViewMode::Xfb => "XFB",
                    })
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut self.view_mode, ViewMode::Auto, "Auto");
                        ui.selectable_value(&mut self.view_mode, ViewMode::Efb, "EFB");
                        ui.selectable_value(&mut self.view_mode, ViewMode::Xfb, "XFB");
                    });

                ui.separator();

                let dirty = self.session.edits.any();
                if ui
                    .add_enabled(dirty, egui::Button::new(format!("{} Save .dff", icons::FLOPPY_DISK)))
                    .on_hover_text("write edits to <name>.edited.dff")
                    .clicked()
                {
                    actions.push(UiAction::Save);
                }

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let s = &self.session;
                    let state = if s.finished {
                        "finished"
                    } else if running {
                        "running"
                    } else {
                        "paused"
                    };

                    ui.label(
                        egui::RichText::new(format!(
                            "{state}  frame {}/{}  cmd {}/{}  presents {}",
                            s.frame_idx,
                            s.frame_count().saturating_sub(1),
                            s.row,
                            s.index.commands.len(),
                            s.presents,
                        ))
                        .monospace(),
                    );

                    if !self.status_msg.is_empty() {
                        ui.separator();
                        ui.label(egui::RichText::new(&self.status_msg).weak());
                    }
                });
            });
        });
    }

    fn frame_panel(&mut self, root: &mut egui::Ui, actions: &mut Vec<UiAction>) {
        egui::Panel::left("frames")
            .resizable(false)
            .exact_size(110.0)
            .show_inside(root, |ui| {
                ui.label(egui::RichText::new("Frames").strong());
                ui.separator();

                let row_h = ui.text_style_height(&egui::TextStyle::Monospace);
                let n = self.session.frame_count();

                let mut scroll = egui::ScrollArea::vertical().auto_shrink([false; 2]);
                if self.scroll_to_current {
                    let spacing = ui.spacing().item_spacing.y;
                    let target = (self.session.frame_idx as f32) * (row_h + spacing) - ui.available_height() * 0.4;

                    scroll = scroll.vertical_scroll_offset(target.max(0.0));
                }

                scroll.show_rows(ui, row_h, n, |ui, range| {
                    for f in range {
                        let current = f == self.session.frame_idx;
                        let in_range = (self.session.start..=self.session.end).contains(&f);

                        let mut text = egui::RichText::new(format!("{f:5}")).monospace();

                        if !in_range {
                            text = text.weak();
                        }
                        if self.session.edits.frame(f).is_some() {
                            text = text.color(egui::Color32::from_rgb(230, 180, 80));
                        }

                        if ui.selectable_label(current, text).clicked() && in_range {
                            actions.push(UiAction::JumpTo(f, 0));
                        }
                    }
                });
            });
    }

    fn command_panel(&mut self, root: &mut egui::Ui, actions: &mut Vec<UiAction>) {
        self.rebuild_rows();

        egui::Panel::left("commands")
            .resizable(true)
            .default_size(480.0)
            .show_inside(root, |ui| {
                ui.label(egui::RichText::new(format!("Frame {} commands", self.session.frame_idx)).strong());

                if self.session.index.truncated {
                    ui.label(
                        egui::RichText::new(format!("{} stream ends mid-command", icons::WARNING))
                            .color(egui::Color32::from_rgb(235, 160, 80)),
                    );
                }
                ui.separator();

                let row_h = ui.text_style_height(&egui::TextStyle::Monospace) + 2.0;
                let total = self.rows.len();

                let mut scroll = egui::ScrollArea::vertical().auto_shrink([false; 2]);
                if self.scroll_to_current {
                    let flat = self
                        .cmd_flat
                        .get(self.session.row)
                        .copied()
                        .unwrap_or(total.saturating_sub(1));
                    let spacing = ui.spacing().item_spacing.y;
                    let target = (flat as f32) * (row_h + spacing) - ui.available_height() * 0.4;

                    scroll = scroll.vertical_scroll_offset(target.max(0.0));
                    self.scroll_to_current = false;
                }

                scroll.show_rows(ui, row_h, total, |ui, range| {
                    for r in range {
                        let row = self.rows[r].clone();
                        self.command_row(ui, &row, actions);
                    }
                });
            });
    }

    fn command_row(&mut self, ui: &mut egui::Ui, row: &Row, actions: &mut Vec<UiAction>) {
        let s = &self.session;
        match *row {
            Row::Update(u) => {
                let update = &s.cur.updates[u];
                let applied = u < s.applied_updates();

                let mut text = egui::RichText::new(format!(
                    "        {} mem {:?} @{:08X} ({} bytes)",
                    icons::MEMORY,
                    update.kind,
                    update.address,
                    update.data.len()
                ))
                .monospace()
                .weak();

                if applied {
                    text = text.color(egui::Color32::from_gray(110));
                }

                ui.label(text);
            }
            Row::FrameEnd => {
                let current = s.at_frame_end();

                let text = egui::RichText::new("── frame end (present) ──").monospace();

                let text = if current {
                    text.color(egui::Color32::from_rgb(255, 220, 120))
                } else {
                    text.weak()
                };

                ui.label(text);
            }
            Row::DlChild { parent, child } => {
                let CmdKind::CallDl {
                    children, phys_addr, ..
                } = &s.index.commands[parent].kind
                else {
                    return;
                };

                let cmd = &children[child];
                let selected = self.selection == Selection::Child(parent, child);
                let label = format!(
                    "    {:08X}  {}",
                    (phys_addr & 0x3FFF_FFFF) as usize + cmd.offset,
                    disasm::summary(cmd)
                );

                if ui
                    .selectable_label(selected, egui::RichText::new(label).monospace().weak())
                    .clicked()
                {
                    self.selection = Selection::Child(parent, child);
                }
            }
            Row::Cmd(i) => {
                let cmd = &s.index.commands[i];
                let off = cmd.offset;
                let is_current = i == s.row && !s.finished;
                let is_selected = self.selection == Selection::Cmd(i);
                let is_disabled = s.cur.disabled.contains(&off);

                let is_edited = s
                    .cur
                    .pristine_offset(off)
                    .map(|orig| {
                        s.edits
                            .frame(s.frame_idx)
                            .is_some_and(|fe| fe.replacements.contains_key(&orig))
                    })
                    .unwrap_or(false);

                let has_bp = s.breakpoints.rows.contains(&(s.frame_idx, i));
                let executed = i < s.row;

                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 4.0;

                    let font_id = egui::FontId::new(
                        ui.style().text_styles[&egui::TextStyle::Body].size,
                        egui::FontFamily::Name("phosphor-fill".into()),
                    );
                    let dot = egui::RichText::new(egui_phosphor::fill::CIRCLE)
                        .font(font_id)
                        .color(if has_bp {
                            egui::Color32::from_rgb(235, 80, 80)
                        } else {
                            egui::Color32::from_gray(60)
                        });

                    if ui.add(egui::Button::new(dot).frame(false).small()).clicked() {
                        actions.push(UiAction::ToggleRowBp(i));
                    }

                    let mut enabled = !is_disabled;
                    if ui.checkbox(&mut enabled, "").changed() {
                        actions.push(UiAction::ToggleDisabled(off));
                    }

                    if let CmdKind::CallDl { children, .. } = &cmd.kind
                        && !children.is_empty()
                    {
                        let arrow = if self.expanded.contains(&off) {
                            icons::CARET_DOWN
                        } else {
                            icons::CARET_RIGHT
                        };

                        if ui.add(egui::Button::new(arrow).frame(false).small()).clicked() {
                            actions.push(UiAction::ToggleExpand(off));
                        }
                    }

                    let mut text = egui::RichText::new(format!(
                        "{off:06X}  {}{}",
                        disasm::summary(cmd),
                        if is_edited { " *" } else { "" }
                    ))
                    .monospace();

                    if is_current {
                        text = text.color(egui::Color32::from_rgb(255, 220, 120));
                    } else if is_disabled {
                        text = text.strikethrough().weak();
                    } else if executed {
                        text = text.weak();
                    }

                    if matches!(cmd.kind, CmdKind::Draw { .. }) && !is_disabled && !executed && !is_current {
                        text = text.color(egui::Color32::from_rgb(140, 200, 255));
                    }

                    if ui.selectable_label(is_selected, text).clicked() {
                        self.selection = Selection::Cmd(i);
                    }
                });
            }
        }
    }

    fn inspector_panel(&mut self, root: &mut egui::Ui, actions: &mut Vec<UiAction>) {
        egui::Panel::right("inspector")
            .resizable(true)
            .default_size(360.0)
            .show_inside(root, |ui| {
                ui.horizontal(|ui| {
                    ui.selectable_value(&mut self.inspector_tab, Tab::Disasm, "Disasm");
                    ui.selectable_value(&mut self.inspector_tab, Tab::Registers, "Registers");
                    ui.selectable_value(&mut self.inspector_tab, Tab::Hex, "Hex");
                    ui.selectable_value(&mut self.inspector_tab, Tab::Breakpoints, "Breakpoints");
                });
                ui.separator();

                egui::ScrollArea::vertical()
                    .auto_shrink([false; 2])
                    .show(ui, |ui| match self.inspector_tab {
                        Tab::Disasm => self.disasm_tab(ui),
                        Tab::Registers => self.registers_tab(ui),
                        Tab::Hex => self.hex_tab(ui, actions),
                        Tab::Breakpoints => self.breakpoints_tab(ui),
                    });
            });
    }

    fn disasm_tab(&self, ui: &mut egui::Ui) {
        let Some(cmd) = self.selected_command() else {
            ui.label(egui::RichText::new("select a command").weak());
            return;
        };
        ui.label(egui::RichText::new(disasm::detail(cmd)).monospace());

        if matches!(self.selection, Selection::Child(..)) {
            ui.separator();
            ui.label(
                egui::RichText::new("display list commands live in RAM and cannot be edited or stepped individually")
                    .weak(),
            );
        }
    }

    fn registers_tab(&self, ui: &mut egui::Ui) {
        let gx = &self.session.playback.gx;

        ui.collapsing("Pipeline state", |ui| {
            let genmode = GenMode::from_raw(gx.bp_regs[BP_GEN_MODE]);
            let z = ZMode::from_raw(gx.bp_regs[BP_PE_ZMODE]);
            let blend = BlendMode::from_raw(gx.bp_regs[BP_PE_CMODE0]);
            let alpha = AlphaCompare::from_raw(gx.bp_regs[BP_PE_ALPHA_COMPARE]);
            ui.label(egui::RichText::new(format!("{genmode:#?}\n{z:#?}\n{blend:#?}\n{alpha:#?}")).monospace());
        });

        ui.collapsing("Textures", |ui| {
            for (i, desc) in gx.cur_textures.iter().enumerate() {
                match desc {
                    Some(d) => ui.label(
                        egui::RichText::new(format!(
                            "{i}: {}x{} {:?} @{:08X}",
                            d.width, d.height, d.format, d.ram_addr
                        ))
                        .monospace(),
                    ),
                    None => ui.label(egui::RichText::new(format!("{i}: -")).monospace().weak()),
                };
            }
        });

        ui.collapsing("CP registers", |ui| {
            for (i, v) in gx.cp_regs.iter().enumerate() {
                if *v != 0 {
                    ui.label(egui::RichText::new(format!("{:18} {v:08X}", disasm::cp_reg_name(i as u8))).monospace());
                }
            }
        });

        ui.collapsing("BP registers", |ui| {
            for (i, v) in gx.bp_regs.iter().enumerate() {
                if *v != 0 {
                    ui.label(egui::RichText::new(format!("{:18} {v:06X}", disasm::bp_reg_name(i as u8))).monospace());
                }
            }
        });

        ui.collapsing("XF registers", |ui| {
            for (i, v) in gx.xf_mem.iter().enumerate().skip(0x1000) {
                if *v != 0 {
                    ui.label(egui::RichText::new(format!("{:18} {v:08X}", disasm::xf_addr_name(i as u16))).monospace());
                }
            }
        });
    }

    fn hex_tab(&mut self, ui: &mut egui::Ui, actions: &mut Vec<UiAction>) {
        let Selection::Cmd(i) = self.selection else {
            ui.label(egui::RichText::new("select a top-level command").weak());
            return;
        };

        let Some(cmd) = self.session.index.commands.get(i) else {
            return;
        };

        let off = cmd.offset;
        let editable = self.session.cur.pristine_offset(off).is_some();

        if self.hex_for != Some(off) {
            self.hex_for = Some(off);
            self.hex_text = self.session.cur.fifo[off..cmd.end()]
                .iter()
                .map(|b| format!("{b:02X}"))
                .collect::<Vec<_>>()
                .join(" ");
        }

        ui.label(egui::RichText::new(format!("command bytes @ {off:06X} ({} bytes)", cmd.len)).monospace());

        ui.add(
            egui::TextEdit::multiline(&mut self.hex_text)
                .font(egui::TextStyle::Monospace)
                .desired_width(f32::INFINITY),
        );

        let parsed = self::parse_hex(&self.hex_text);

        ui.horizontal(|ui| {
            match &parsed {
                Ok(bytes) => {
                    ui.label(egui::RichText::new(format!("{} bytes", bytes.len())).weak());
                }
                Err(err) => {
                    ui.label(egui::RichText::new(err.as_str()).color(egui::Color32::from_rgb(235, 80, 80)));
                }
            }

            if ui
                .add_enabled(editable && parsed.is_ok(), egui::Button::new("Apply"))
                .clicked()
                && let Ok(bytes) = parsed
            {
                actions.push(UiAction::ApplyHex(off, bytes));
            }

            if ui.add_enabled(editable, egui::Button::new("Revert")).clicked() {
                actions.push(UiAction::Revert(off));
            }
        });

        if !editable {
            ui.label(
                egui::RichText::new("this command cannot be mapped back to a pristine boundary, editing disabled")
                    .weak(),
            );
        }

        ui.label(
            egui::RichText::new(
                "size changes are allowed: later offsets shift and memory update positions are remapped",
            )
            .weak(),
        );
    }

    fn breakpoints_tab(&mut self, ui: &mut egui::Ui) {
        let bp = &mut self.session.breakpoints;

        ui.label(egui::RichText::new("Break on command kind").strong());
        ui.checkbox(&mut bp.on_draw, "draw");
        ui.checkbox(&mut bp.on_calldl, "display list call");
        ui.checkbox(&mut bp.on_efb_copy, "EFB copy execute");
        ui.checkbox(&mut bp.on_cp, "any CP write");
        ui.checkbox(&mut bp.on_bp, "any BP write");
        ui.checkbox(&mut bp.on_xf, "any XF write");

        ui.separator();
        ui.label(egui::RichText::new("Register breakpoints").strong());
        ui.label(egui::RichText::new("cp <hex> | bp <hex> | xf <hex>").weak());

        ui.horizontal(|ui| {
            ui.add(egui::TextEdit::singleline(&mut self.bp_input).desired_width(120.0));

            if ui.button(format!("{} Add", icons::PLUS)).clicked() {
                let input = self.bp_input.trim().to_lowercase();
                let mut parts = input.split_whitespace();
                let kind = parts.next().unwrap_or("");
                let val = parts.next().and_then(|v| u16::from_str_radix(v, 16).ok());

                match (kind, val) {
                    ("cp", Some(v)) if v <= 0xFF => {
                        bp.cp_regs.insert(v as u8);
                        self.bp_input.clear();
                    }
                    ("bp", Some(v)) if v <= 0xFF => {
                        bp.bp_regs.insert(v as u8);
                        self.bp_input.clear();
                    }
                    ("xf", Some(v)) => {
                        bp.xf_addrs.insert(v);
                        self.bp_input.clear();
                    }
                    _ => self.status_msg = "expected: cp|bp|xf <hex>".into(),
                }
            }
        });

        self::breakpoint_list(ui, &mut bp.cp_regs, |reg| format!("CP {}", disasm::cp_reg_name(reg)));
        self::breakpoint_list(ui, &mut bp.bp_regs, |reg| format!("BP {}", disasm::bp_reg_name(reg)));
        self::breakpoint_list(ui, &mut bp.xf_addrs, |addr| {
            format!("XF {}", disasm::xf_addr_name(addr))
        });

        if !bp.rows.is_empty() {
            ui.separator();
            ui.label(egui::RichText::new("Command breakpoints").strong());

            let mut remove_row = None;
            let mut rows: Vec<_> = bp.rows.iter().copied().collect();
            rows.sort();

            for (frame, row) in rows {
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new(format!("frame {frame} cmd {row}")).monospace());
                    if ui.small_button(icons::TRASH).clicked() {
                        remove_row = Some((frame, row));
                    }
                });
            }

            if let Some(key) = remove_row {
                bp.rows.remove(&key);
            }
        }
    }

    fn game_panel(&mut self, root: &mut egui::Ui) {
        egui::CentralPanel::default().show_inside(root, |ui| {
            let show_xfb = match self.view_mode {
                ViewMode::Xfb => true,
                ViewMode::Efb => false,
                ViewMode::Auto => self.session.row == 0 && self.session.presents > 0,
            };

            let (tex, size, label) = if show_xfb {
                (
                    self.xfb_tex,
                    egui::vec2(self.xfb_size.0 as f32, self.xfb_size.1 as f32),
                    "XFB",
                )
            } else {
                (
                    self.efb_tex,
                    egui::vec2(
                        gecko::flipper::gx::constants::EFB_WIDTH as f32,
                        gecko::flipper::gx::constants::EFB_HEIGHT as f32,
                    ),
                    "EFB",
                )
            };

            ui.label(egui::RichText::new(label).weak());

            let Some(tex) = tex else { return };
            let avail = ui.available_size();
            let scale = (avail.x / size.x).min(avail.y / size.y).max(0.01);
            let draw_size = egui::vec2(size.x * scale, size.y * scale);

            ui.centered_and_justified(|ui| {
                ui.image((tex, draw_size));
            });
        });
    }

    fn render(&mut self) {
        let Some(window) = self.window.clone() else { return };
        let (frame, width, height) = {
            let Some((surface, config)) = self.surface.as_ref() else {
                return;
            };
            let frame = match surface.get_current_texture() {
                wgpu::CurrentSurfaceTexture::Success(f) | wgpu::CurrentSurfaceTexture::Suboptimal(f) => f,
                status => {
                    tracing::error!(?status, "surface error");
                    return;
                }
            };
            (frame, config.width, config.height)
        };
        let view = frame.texture.create_view(&Default::default());

        let raw_input = self.egui_winit.as_mut().unwrap().take_egui_input(&window);
        let egui_ctx = self.egui_ctx.clone();
        let mut actions = Vec::new();
        let full_output = egui_ctx.run_ui(raw_input, |ui| {
            self.build_ui(ui, &mut actions);
        });

        self.egui_winit
            .as_mut()
            .unwrap()
            .handle_platform_output(&window, full_output.platform_output);

        for action in actions {
            self.apply_action(action);
        }
        self.refresh_xfb_binding();

        let screen_desc = egui_wgpu::ScreenDescriptor {
            size_in_pixels: [width, height],
            pixels_per_point: window.scale_factor() as f32,
        };
        let tris = self
            .egui_ctx
            .tessellate(full_output.shapes, screen_desc.pixels_per_point);

        let egui_renderer = self.egui_renderer.as_mut().unwrap();

        for (id, delta) in full_output.textures_delta.set {
            egui_renderer.update_texture(&self.device, &self.queue, id, &delta);
        }

        let mut encoder = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("debug_egui"),
        });
        egui_renderer.update_buffers(&self.device, &self.queue, &mut encoder, &tris, &screen_desc);
        {
            let rpass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("debug_egui"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: 0.07,
                            g: 0.07,
                            b: 0.08,
                            a: 1.0,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                    depth_slice: None,
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            egui_renderer.render(&mut rpass.forget_lifetime(), &tris, &screen_desc);
        }
        self.queue.submit([encoder.finish()]);

        for id in full_output.textures_delta.free {
            self.egui_renderer.as_mut().unwrap().free_texture(&id);
        }

        frame.present();

        let wants_repaint = full_output
            .viewport_output
            .get(&ViewportId::ROOT)
            .is_some_and(|v| v.repaint_delay.is_zero());
        if self.session.run_state != RunState::Paused || wants_repaint {
            window.request_redraw();
        }
    }
}

fn breakpoint_list<T: Copy + Eq + Ord + std::hash::Hash>(
    ui: &mut egui::Ui,
    set: &mut HashSet<T>,
    name: impl Fn(T) -> String,
) {
    let mut items: Vec<T> = set.iter().copied().collect();
    items.sort();

    let mut remove = None;

    for item in items {
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new(name(item)).monospace());
            if ui.small_button(icons::TRASH).clicked() {
                remove = Some(item);
            }
        });
    }
    if let Some(item) = remove {
        set.remove(&item);
    }
}

fn parse_hex(text: &str) -> Result<Vec<u8>, String> {
    let cleaned: String = text.chars().filter(|c| !c.is_whitespace()).collect();

    if cleaned.is_empty() {
        return Err("empty".into());
    }
    if cleaned.len() % 2 != 0 {
        return Err("odd number of hex digits".into());
    }

    (0..cleaned.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&cleaned[i..i + 2], 16).map_err(|_| "invalid hex".to_string()))
        .collect()
}

impl<const SYSTEM: SystemId> ApplicationHandler for DebugApp<SYSTEM> {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        let window = Arc::new(
            event_loop
                .create_window(
                    Window::default_attributes()
                        .with_title("Gecko — FIFO Debugger")
                        .with_inner_size(winit::dpi::PhysicalSize::new(1760, 990)),
                )
                .unwrap(),
        );

        let surface = self.instance.create_surface(window.clone()).unwrap();
        let size = window.inner_size();

        let surface_caps = surface.get_capabilities(&self.adapter);
        let surface_config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format: SURFACE_FORMAT,
            width: size.width.max(1),
            height: size.height.max(1),
            present_mode: wgpu::PresentMode::Fifo,
            alpha_mode: surface_caps.alpha_modes[0],
            view_formats: vec![],
            desired_maximum_frame_latency: 2,
        };

        surface.configure(&self.device, &surface_config);

        let mut egui_renderer =
            egui_wgpu::Renderer::new(&self.device, SURFACE_FORMAT, egui_wgpu::RendererOptions::default());
        let egui_winit = egui_winit::State::new(
            self.egui_ctx.clone(),
            ViewportId::ROOT,
            window.as_ref(),
            None,
            None,
            None,
        );

        {
            let g = self.gx.lock().unwrap();
            self.efb_tex =
                Some(egui_renderer.register_native_texture(&self.device, g.efb_view(), wgpu::FilterMode::Linear));
            self.xfb_tex =
                Some(egui_renderer.register_native_texture(&self.device, &g.xfb_view, wgpu::FilterMode::Linear));
            let size = g.xfb_texture.size();
            self.xfb_size = (size.width, size.height);
        }

        self.flush_gx();

        window.request_redraw();

        self.window = Some(window);
        self.surface = Some((surface, surface_config));
        self.egui_winit = Some(egui_winit);
        self.egui_renderer = Some(egui_renderer);
    }

    fn exiting(&mut self, _event_loop: &ActiveEventLoop) {
        self.egui_winit = None;
        self.egui_renderer = None;
        self.surface = None;
        self.window = None;
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        if let (Some(state), Some(window)) = (self.egui_winit.as_mut(), self.window.as_ref()) {
            let response = state.on_window_event(window, &event);

            if response.repaint {
                window.request_redraw();
            }
            if response.consumed {
                return;
            }
        }

        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => {
                if size.width == 0 || size.height == 0 {
                    return;
                }

                if let Some((surface, config)) = &mut self.surface {
                    config.width = size.width;
                    config.height = size.height;

                    surface.configure(&self.device, config);
                }
            }
            WindowEvent::RedrawRequested => {
                if self.session.run_state != RunState::Paused {
                    self.session.run_tick();
                    self.flush_gx();
                    self.scroll_to_current = true;
                }

                self.render();
            }
            _ => {}
        }
    }
}
