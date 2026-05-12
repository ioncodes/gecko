use std::path::PathBuf;
use std::sync::Arc;

use base64::Engine;
use base64::engine::general_purpose::STANDARD as B64;
use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{CallToolResult, Content, Implementation, ProtocolVersion, ServerCapabilities, ServerInfo};
use rmcp::{ErrorData as McpError, ServerHandler, schemars, tool, tool_handler, tool_router};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::boot;
use crate::sink::Introspection;
use crate::state::{Backend, RunMode, Shared};

const MAX_READ_LEN: u32 = 1 << 20;
const MAX_DISASM_COUNT: u32 = 4096;
const MAX_FRAMES_PER_CALL: u32 = 600;
const MAX_STEP_COUNT: u32 = 1 << 20;

#[derive(Clone)]
pub struct McpServer {
    shared: Arc<Shared>,
    #[allow(dead_code)]
    tool_router: ToolRouter<McpServer>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct LoadGameArgs {
    #[schemars(description = "Filesystem path to a GameCube/Wii ISO/RVZ/ZIP image")]
    pub path: Option<String>,
    #[schemars(description = "Base64-encoded disc image bytes (alternative to path)")]
    pub bytes_b64: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct StepArgs {
    #[schemars(description = "Number of PPC instructions to step (default 1)")]
    pub count: Option<u32>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct RunFramesArgs {
    #[schemars(description = "Number of vsyncs to advance, 1..=600")]
    pub count: u32,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct RunUntilPcArgs {
    #[schemars(description = "Target program counter (virtual address)")]
    pub pc: u32,
    #[schemars(description = "Cycle budget; bail out and report a timeout if exceeded")]
    pub max_cycles: Option<u64>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ReadMemoryArgs {
    pub addr: u32,
    pub len: u32,
    #[schemars(description = "Treat addr as a virtual address (default true)")]
    pub virt: Option<bool>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct WriteMemoryArgs {
    pub addr: u32,
    #[schemars(description = "Base64 of bytes to write")]
    pub bytes_b64: String,
    pub virt: Option<bool>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct SearchBytesArgs {
    pub addr: u32,
    pub len: u32,
    #[schemars(description = "Base64 of needle bytes")]
    pub needle_b64: String,
    #[schemars(description = "Maximum number of hits to return (default 32)")]
    pub max_hits: Option<u32>,
    pub virt: Option<bool>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct DisassembleArgs {
    pub addr: u32,
    #[schemars(description = "Number of instructions to decode (1..=4096)")]
    pub count: u32,
    pub virt: Option<bool>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct GetTextureArgs {
    pub ram_addr: u32,
    #[schemars(description = "Variant hash; 0 for non-paletted (default 0)")]
    pub variant: Option<u32>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct SetPadArgs {
    pub stick_x: Option<u8>,
    pub stick_y: Option<u8>,
    pub substick_x: Option<u8>,
    pub substick_y: Option<u8>,
    pub trigger_left: Option<u8>,
    pub trigger_right: Option<u8>,
    #[schemars(description = "GameCube pad button bitmask (A=0x100, B=0x200, START=0x1000, ...)")]
    pub buttons: Option<u16>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct SetWiimoteArgs {
    #[schemars(
        description = "Wiimote core button bitmask: A=0x0008, B=0x0004, ONE=0x0002, TWO=0x0001, MINUS=0x0010, HOME=0x0080, PLUS=0x1000, LEFT=0x0100, RIGHT=0x0200, DOWN=0x0400, UP=0x0800"
    )]
    pub buttons: Option<u16>,
    #[schemars(
        description = "Simulate shaking the Wiimote. While true, the Wiimote core accelerometer reports a 2g, 10 Hz sinusoid on all axes (on top of the +1g Z gravity baseline), which games detect as a shake gesture."
    )]
    pub shake: Option<bool>,
    #[schemars(description = "Nunchuk button bitmask: Z=0x01, C=0x02")]
    pub nunchuk_buttons: Option<u8>,
    #[schemars(description = "Nunchuk stick X (0..=255, center=0x80, full-left=0x00, full-right=0xFF)")]
    pub nunchuk_stick_x: Option<u8>,
    #[schemars(description = "Nunchuk stick Y (0..=255, center=0x80, full-down=0x00, full-up=0xFF)")]
    pub nunchuk_stick_y: Option<u8>,
    #[schemars(
        description = "IR pointer X in camera space (0..=1023). Both ir_x and ir_y must be set to enable the pointer; omitting either reports it as not visible."
    )]
    pub ir_x: Option<u16>,
    #[schemars(description = "IR pointer Y in camera space (0..=767).")]
    pub ir_y: Option<u16>,
}

#[derive(Debug, Serialize)]
struct StatusJson {
    loaded: bool,
    system: Option<&'static str>,
    game_name: String,
    game_code: String,
    run_mode: &'static str,
    pc: Option<u32>,
    cycles: Option<u64>,
    frame_index: u64,
    last_xfb_size: (u32, u32),
    draw_count_last_frame: u64,
    texture_count: usize,
}

#[derive(Debug, Serialize)]
struct RegistersJson {
    pc: u32,
    cia: u32,
    nia: u32,
    lr: u32,
    ctr: u32,
    msr: u32,
    cr: u32,
    xer: u32,
    srr0: u32,
    srr1: u32,
    gprs: [u32; 32],
    fprs: [f64; 32],
    ps1s: [f64; 32],
}

#[derive(Debug, Serialize)]
struct DisasmLine {
    addr: u32,
    raw: u32,
    text: String,
}

#[derive(Debug, Serialize)]
struct TextureSummary {
    ram_addr: u32,
    variant: u32,
    width: u32,
    height: u32,
    format: &'static str,
    last_seen_frame: u64,
    bound_slots: Vec<u8>,
}

impl McpServer {
    pub fn new(shared: Arc<Shared>) -> Self {
        Self {
            shared,
            tool_router: Self::tool_router(),
        }
    }
}

#[tool_router]
impl McpServer {
    #[tool(
        description = "Report whether a game is loaded plus current execution state, frame index, and texture count."
    )]
    async fn status(&self) -> Result<CallToolResult, McpError> {
        let s = self.shared.state.lock().unwrap();
        let intro = self.shared.introspect.lock().unwrap();
        let (loaded, system, pc, cycles) = match s.backend.as_ref() {
            None => (false, None, None, None),
            Some(b) => {
                let sys = match b {
                    Backend::Gc(_) => "gamecube",
                    Backend::Wii(_) => "wii",
                };
                (true, Some(sys), Some(b.pc()), Some(b.cycles()))
            }
        };
        let json = StatusJson {
            loaded,
            system,
            game_name: s.game_name.clone(),
            game_code: s.game_code.clone(),
            run_mode: match s.run_mode {
                RunMode::Paused => "paused",
                RunMode::Running => "running",
            },
            pc,
            cycles,
            frame_index: intro.frame_index,
            last_xfb_size: intro.last_xfb_size,
            draw_count_last_frame: intro.draw_count_this_frame,
            texture_count: intro.textures.len(),
        };
        ok_json(&json)
    }

    #[tool(
        description = "Load a GameCube or Wii disc image. Provide either a filesystem path or base64 bytes. The system kind is detected from the disc header. The emulator pauses on load; call resume or run_frames to advance."
    )]
    async fn load_game(&self, Parameters(args): Parameters<LoadGameArgs>) -> Result<CallToolResult, McpError> {
        let bytes = match (args.path.as_ref(), args.bytes_b64.as_ref()) {
            (Some(path), None) => std::fs::read(PathBuf::from(path))
                .map_err(|e| McpError::invalid_params(format!("read disc: {e}"), None))?,
            (None, Some(b64)) => B64
                .decode(b64.as_bytes())
                .map_err(|e| McpError::invalid_params(format!("decode bytes_b64: {e}"), None))?,
            _ => {
                return Err(McpError::invalid_params(
                    "supply exactly one of path or bytes_b64",
                    None,
                ));
            }
        };

        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            boot::boot(
                bytes,
                &self.shared.ipl,
                &self.shared.dsp_rom,
                &self.shared.coef_rom,
                self.shared.gx.clone(),
                self.shared.device.clone(),
                self.shared.queue.clone(),
                self.shared.introspect.clone(),
            )
        }))
        .map_err(|_| McpError::internal_error("emulator boot panicked", None))?;

        let mut s = self.shared.state.lock().unwrap();
        s.backend = Some(result.backend);
        s.game_name = result.game_name.clone();
        s.game_code = result.game_code.clone();
        s.run_mode = RunMode::Paused;
        let mut intro = self.shared.introspect.lock().unwrap();
        *intro = Introspection::default();
        drop(intro);
        drop(s);
        self.shared.cv.notify_all();
        ok_json(&json!({ "loaded": true, "game_name": result.game_name, "game_code": result.game_code }))
    }

    #[tool(
        description = "Switch to Running mode. The emulator advances one vsync per loop iteration on the worker thread until pause is called."
    )]
    async fn resume(&self) -> Result<CallToolResult, McpError> {
        require_loaded(&self.shared)?;
        self.shared.set_run_mode(RunMode::Running);
        ok_json(&json!({"run_mode": "running"}))
    }

    #[tool(
        description = "Switch to Paused mode. Other tools (step, run_frames, run_until_pc, read_memory, ...) only work while paused."
    )]
    async fn pause(&self) -> Result<CallToolResult, McpError> {
        self.shared.set_run_mode(RunMode::Paused);
        let pc = self.shared.state.lock().unwrap().backend.as_ref().map(|b| b.pc());
        ok_json(&json!({"run_mode": "paused", "pc": pc}))
    }

    #[tool(
        description = "Advance the emulator by exactly N PPC instructions (no scheduler events between, just step_cpu). Requires Paused."
    )]
    async fn step(&self, Parameters(args): Parameters<StepArgs>) -> Result<CallToolResult, McpError> {
        require_loaded(&self.shared)?;
        require_paused(&self.shared)?;
        let count = args.count.unwrap_or(1).min(MAX_STEP_COUNT);
        let mut s = self.shared.state.lock().unwrap();
        let backend = s.backend.as_mut().unwrap();
        for _ in 0..count {
            backend.step();
        }
        ok_json(&json!({"stepped": count, "pc": backend.pc(), "cycles": backend.cycles()}))
    }

    #[tool(
        description = "Advance the emulator by N full frames (vsyncs). Each frame runs the normal scheduler / GPU pipeline. Capped at 600 per call."
    )]
    async fn run_frames(&self, Parameters(args): Parameters<RunFramesArgs>) -> Result<CallToolResult, McpError> {
        require_loaded(&self.shared)?;
        require_paused(&self.shared)?;
        let count = args.count.clamp(1, MAX_FRAMES_PER_CALL);
        let mut s = self.shared.state.lock().unwrap();
        let backend = s.backend.as_mut().unwrap();
        for _ in 0..count {
            backend.run_until_vsync();
        }
        let intro = self.shared.introspect.lock().unwrap();
        ok_json(&json!({
            "frames": count,
            "pc": backend.pc(),
            "cycles": backend.cycles(),
            "frame_index": intro.frame_index,
        }))
    }

    #[tool(
        description = "Step instructions until the program counter reaches `pc`, or `max_cycles` (default 100M) elapses. Reports whether the target was hit."
    )]
    async fn run_until_pc(&self, Parameters(args): Parameters<RunUntilPcArgs>) -> Result<CallToolResult, McpError> {
        require_loaded(&self.shared)?;
        require_paused(&self.shared)?;
        let max = args.max_cycles.unwrap_or(100_000_000);
        let mut s = self.shared.state.lock().unwrap();
        let backend = s.backend.as_mut().unwrap();
        let start = backend.cycles();
        let mut hit = false;
        loop {
            if backend.pc() == args.pc {
                hit = true;
                break;
            }
            if backend.cycles().wrapping_sub(start) > max {
                break;
            }
            backend.step();
        }
        ok_json(&json!({
            "hit": hit,
            "pc": backend.pc(),
            "cycles_elapsed": backend.cycles().wrapping_sub(start),
        }))
    }

    #[tool(description = "Run the emulator until the next vsync (one frame). Convenience for run_frames(1).")]
    async fn run_until_vsync(&self) -> Result<CallToolResult, McpError> {
        require_loaded(&self.shared)?;
        require_paused(&self.shared)?;
        let mut s = self.shared.state.lock().unwrap();
        let backend = s.backend.as_mut().unwrap();
        backend.run_until_vsync();
        ok_json(&json!({"pc": backend.pc(), "cycles": backend.cycles()}))
    }

    #[tool(
        description = "Read len bytes of GameCube/Wii memory at addr (virtual by default). Returns base64. len capped at 1 MiB."
    )]
    async fn read_memory(&self, Parameters(args): Parameters<ReadMemoryArgs>) -> Result<CallToolResult, McpError> {
        require_loaded(&self.shared)?;
        let len = args.len.min(MAX_READ_LEN) as usize;
        let s = self.shared.state.lock().unwrap();
        let bytes = read_bytes(s.backend.as_ref().unwrap(), args.addr, len, args.virt.unwrap_or(true))?;
        ok_json(&json!({
            "addr": args.addr,
            "len": bytes.len(),
            "bytes_b64": B64.encode(&bytes),
        }))
    }

    #[tool(description = "Write base64 bytes to memory at addr. Use sparingly; the emulator does not validate writes.")]
    async fn write_memory(&self, Parameters(args): Parameters<WriteMemoryArgs>) -> Result<CallToolResult, McpError> {
        require_loaded(&self.shared)?;
        let bytes = B64
            .decode(args.bytes_b64.as_bytes())
            .map_err(|e| McpError::invalid_params(format!("decode bytes_b64: {e}"), None))?;
        let mut s = self.shared.state.lock().unwrap();
        write_bytes(
            s.backend.as_mut().unwrap(),
            args.addr,
            &bytes,
            args.virt.unwrap_or(true),
        )?;
        ok_json(&json!({"addr": args.addr, "len": bytes.len()}))
    }

    #[tool(description = "Search a memory range for a needle. Returns the first N hit addresses.")]
    async fn search_bytes(&self, Parameters(args): Parameters<SearchBytesArgs>) -> Result<CallToolResult, McpError> {
        require_loaded(&self.shared)?;
        let len = args.len.min(MAX_READ_LEN) as usize;
        let needle = B64
            .decode(args.needle_b64.as_bytes())
            .map_err(|e| McpError::invalid_params(format!("decode needle_b64: {e}"), None))?;
        if needle.is_empty() {
            return Err(McpError::invalid_params("needle is empty", None));
        }
        let max_hits = args.max_hits.unwrap_or(32) as usize;
        let s = self.shared.state.lock().unwrap();
        let buf = read_bytes(s.backend.as_ref().unwrap(), args.addr, len, args.virt.unwrap_or(true))?;
        let mut hits = Vec::new();
        for (i, w) in buf.windows(needle.len()).enumerate() {
            if w == needle.as_slice() {
                hits.push(args.addr.wrapping_add(i as u32));
                if hits.len() >= max_hits {
                    break;
                }
            }
        }
        ok_json(&json!({"hits": hits, "count": hits.len()}))
    }

    #[tool(description = "Read GPRs, FPRs, PS1s, PC/CIA/NIA, LR, CTR, MSR, CR, XER, SRR0, SRR1.")]
    async fn get_registers(&self) -> Result<CallToolResult, McpError> {
        require_loaded(&self.shared)?;
        let s = self.shared.state.lock().unwrap();
        let regs = match s.backend.as_ref().unwrap() {
            Backend::Gc(sys) => registers_of(sys),
            Backend::Wii(sys) => registers_of(sys),
        };
        ok_json(&regs)
    }

    #[tool(description = "Disassemble PowerPC instructions starting at addr. Returns [{addr, raw, text}, ...].")]
    async fn disassemble(&self, Parameters(args): Parameters<DisassembleArgs>) -> Result<CallToolResult, McpError> {
        require_loaded(&self.shared)?;
        let count = args.count.min(MAX_DISASM_COUNT);
        let virt = args.virt.unwrap_or(true);
        let s = self.shared.state.lock().unwrap();
        let backend = s.backend.as_ref().unwrap();
        let mut lines = Vec::with_capacity(count as usize);
        for i in 0..count {
            let addr = args.addr.wrapping_add(i.wrapping_mul(4));
            let raw = read_u32(backend, addr, virt)?;
            let text = disasm::gekko::GekkoInstruction::decode(&raw.to_be_bytes())
                .map(|(insn, _)| insn.to_string())
                .unwrap_or_else(|| format!(".word {:#010X}", raw));
            lines.push(DisasmLine { addr, raw, text });
        }
        ok_json(&lines)
    }

    #[tool(description = "Capture the current frame from the emulator's XFB and return it as a base64 PNG.")]
    async fn get_view(&self) -> Result<CallToolResult, McpError> {
        require_loaded(&self.shared)?;
        let _ = self.shared.device.poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: None,
        });
        let captured = {
            let gx = self.shared.gx.lock().unwrap();
            backend_wgpu::capture::capture_texture(&self.shared.device, &self.shared.queue, &gx.xfb_texture)
                .ok_or_else(|| McpError::internal_error("capture_texture returned None", None))?
        };
        let mut rgba = captured.rgba;
        for px in rgba.chunks_exact_mut(4) {
            px[3] = 255;
        }
        let png = encode_png(captured.width, captured.height, &rgba)?;
        ok_json(&json!({
            "width": captured.width,
            "height": captured.height,
            "png_b64": B64.encode(&png),
        }))
    }

    #[tool(
        description = "List all GX textures the emulator has uploaded so far this session, with metadata. Returns at most 1024 entries."
    )]
    async fn list_textures(&self) -> Result<CallToolResult, McpError> {
        let intro = self.shared.introspect.lock().unwrap();
        let mut out: Vec<TextureSummary> = intro
            .textures
            .iter()
            .map(|(key, rec)| {
                let mut bound_slots = Vec::new();
                for (i, b) in intro.bound.iter().enumerate() {
                    if b.as_ref() == Some(key) {
                        bound_slots.push(i as u8);
                    }
                }
                TextureSummary {
                    ram_addr: key.ram_addr,
                    variant: key.variant,
                    width: rec.width,
                    height: rec.height,
                    format: format_name(rec.format),
                    last_seen_frame: rec.last_seen_frame,
                    bound_slots,
                }
            })
            .collect();
        out.sort_by_key(|t| (t.ram_addr, t.variant));
        out.truncate(1024);
        ok_json(&json!({"textures": out, "count": intro.textures.len()}))
    }

    #[tool(
        description = "Return one previously uploaded GX texture as a base64 PNG. Identify by ram_addr (and variant for paletted textures)."
    )]
    async fn get_texture(&self, Parameters(args): Parameters<GetTextureArgs>) -> Result<CallToolResult, McpError> {
        let intro = self.shared.introspect.lock().unwrap();
        let key = gecko::host::TextureKey {
            ram_addr: args.ram_addr,
            variant: args.variant.unwrap_or(0),
        };
        let rec = intro.textures.get(&key).ok_or_else(|| {
            McpError::invalid_params(
                format!(
                    "no texture at ram_addr={:#010X} variant={:#010X}",
                    args.ram_addr,
                    args.variant.unwrap_or(0)
                ),
                None,
            )
        })?;
        let png = encode_png(rec.width, rec.height, &rec.rgba)?;
        ok_json(&json!({
            "ram_addr": args.ram_addr,
            "variant": key.variant,
            "width": rec.width,
            "height": rec.height,
            "format": format_name(rec.format),
            "png_b64": B64.encode(&png),
        }))
    }

    #[tool(description = "Return the TextureKey currently bound to each of the 8 GX texture slots. null = unbound.")]
    async fn list_bound_textures(&self) -> Result<CallToolResult, McpError> {
        let intro = self.shared.introspect.lock().unwrap();
        let bound: Vec<_> = intro
            .bound
            .iter()
            .map(|b| b.map(|k| json!({"ram_addr": k.ram_addr, "variant": k.variant})))
            .collect();
        ok_json(&json!({"bound": bound}))
    }

    #[tool(description = "Update the GameCube controller (port 1) state. Unspecified fields stay at their last value.")]
    async fn set_pad(&self, Parameters(args): Parameters<SetPadArgs>) -> Result<CallToolResult, McpError> {
        require_loaded(&self.shared)?;
        let mut pad = gecko::flipper::si::pad::PadStatus::default();
        pad.connected = true;
        if let Some(v) = args.stick_x {
            pad.stick_x = v;
        }
        if let Some(v) = args.stick_y {
            pad.stick_y = v;
        }
        if let Some(v) = args.substick_x {
            pad.substick_x = v;
        }
        if let Some(v) = args.substick_y {
            pad.substick_y = v;
        }
        if let Some(v) = args.trigger_left {
            pad.trigger_left = v;
        }
        if let Some(v) = args.trigger_right {
            pad.trigger_right = v;
        }
        if let Some(v) = args.buttons {
            pad.buttons = v;
        }
        let input = gecko::HostInput::Gc(pad);
        let mut s = self.shared.state.lock().unwrap();
        s.backend.as_mut().unwrap().apply_host_input(&input);
        ok_json(&json!({"applied": true}))
    }

    #[tool(
        description = "Update the Wii remote + nunchuk state. Defaults: buttons=0, stick centered (0x80). Unspecified fields go to those defaults."
    )]
    async fn set_wiimote(&self, Parameters(args): Parameters<SetWiimoteArgs>) -> Result<CallToolResult, McpError> {
        require_loaded(&self.shared)?;
        let input = gecko::HostInput::Wii {
            wiimote_buttons: args.buttons.unwrap_or(0),
            wiimote_shake: args.shake.unwrap_or(false),
            nunchuk_buttons: args.nunchuk_buttons.unwrap_or(0),
            nunchuk_stick_x: args
                .nunchuk_stick_x
                .unwrap_or(gecko::hollywood::ipc::usb::NUNCHUK_STICK_CENTER),
            nunchuk_stick_y: args
                .nunchuk_stick_y
                .unwrap_or(gecko::hollywood::ipc::usb::NUNCHUK_STICK_CENTER),
            ir_pointer: args.ir_x.zip(args.ir_y),
        };
        let mut s = self.shared.state.lock().unwrap();
        s.backend.as_mut().unwrap().apply_host_input(&input);
        ok_json(&json!({"applied": true}))
    }
}

#[tool_handler]
impl ServerHandler for McpServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::from_build_env())
            .with_protocol_version(ProtocolVersion::V_2024_11_05)
            .with_instructions(
                "Headless gecko (GameCube/Wii) emulator. Tools: load_game, status, pause/resume, step, run_frames, run_until_pc, run_until_vsync, read_memory, write_memory, search_bytes, get_registers, disassemble, get_view, list_textures, get_texture, list_bound_textures, set_pad, set_wiimote."
                    .to_string(),
            )
    }
}

fn require_loaded(shared: &Shared) -> Result<(), McpError> {
    if shared.state.lock().unwrap().backend.is_some() {
        Ok(())
    } else {
        Err(McpError::invalid_request(
            "no game is loaded; call load_game first",
            None,
        ))
    }
}

fn require_paused(shared: &Shared) -> Result<(), McpError> {
    let s = shared.state.lock().unwrap();
    if s.run_mode == RunMode::Paused {
        Ok(())
    } else {
        Err(McpError::invalid_request(
            "the emulator is running; call pause first",
            None,
        ))
    }
}

fn read_bytes(backend: &Backend, addr: u32, len: usize, virt: bool) -> Result<Vec<u8>, McpError> {
    let mut out = Vec::with_capacity(len);
    for i in 0..len {
        let a = addr.wrapping_add(i as u32);
        let byte = match backend {
            Backend::Gc(s) => {
                if virt {
                    s.mmio.virt_read_u8(a)
                } else {
                    s.mmio.phys_read_u8(a)
                }
            }
            Backend::Wii(s) => {
                if virt {
                    s.mmio.virt_read_u8(a)
                } else {
                    s.mmio.phys_read_u8(a)
                }
            }
        };
        out.push(byte);
    }
    Ok(out)
}

fn write_bytes(backend: &mut Backend, addr: u32, bytes: &[u8], virt: bool) -> Result<(), McpError> {
    for (i, b) in bytes.iter().enumerate() {
        let a = addr.wrapping_add(i as u32);
        match backend {
            Backend::Gc(s) => {
                if virt {
                    s.mmio.virt_write_u8(a, *b);
                } else {
                    s.mmio.phys_write_u8(a, *b);
                }
            }
            Backend::Wii(s) => {
                if virt {
                    s.mmio.virt_write_u8(a, *b);
                } else {
                    s.mmio.phys_write_u8(a, *b);
                }
            }
        }
    }
    Ok(())
}

fn read_u32(backend: &Backend, addr: u32, virt: bool) -> Result<u32, McpError> {
    Ok(match backend {
        Backend::Gc(s) => {
            if virt {
                s.mmio.virt_read_u32(addr)
            } else {
                s.mmio.phys_read_u32(addr)
            }
        }
        Backend::Wii(s) => {
            if virt {
                s.mmio.virt_read_u32(addr)
            } else {
                s.mmio.phys_read_u32(addr)
            }
        }
    })
}

fn registers_of<const SYSTEM: gecko::SystemId>(sys: &gecko::System<{ SYSTEM }>) -> RegistersJson {
    let g = &sys.gekko;
    RegistersJson {
        pc: g.pc,
        cia: g.cia,
        nia: g.nia,
        lr: g.spr.lr,
        ctr: g.spr.ctr,
        msr: g.msr.raw(),
        cr: g.cr.raw(),
        xer: g.spr.xer.raw(),
        srr0: g.spr.srr0.raw(),
        srr1: g.spr.srr1,
        gprs: g.gprs,
        fprs: g.fprs,
        ps1s: g.ps1s,
    }
}

fn format_name(fmt: gecko::flipper::gx::draw::TextureFormat) -> &'static str {
    use gecko::flipper::gx::draw::TextureFormat as TF;
    match fmt {
        TF::I4 => "I4",
        TF::I8 => "I8",
        TF::IA4 => "IA4",
        TF::IA8 => "IA8",
        TF::RGB565 => "RGB565",
        TF::RGB5A3 => "RGB5A3",
        TF::RGBA8 => "RGBA8",
        TF::CI4 => "CI4",
        TF::CI8 => "CI8",
        TF::CI14 => "CI14",
        TF::CMPR => "CMPR",
    }
}

fn encode_png(width: u32, height: u32, rgba: &[u8]) -> Result<Vec<u8>, McpError> {
    let mut buf = Vec::with_capacity(rgba.len());
    {
        let mut encoder = png::Encoder::new(&mut buf, width, height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder
            .write_header()
            .map_err(|e| McpError::internal_error(format!("png header: {e}"), None))?;
        writer
            .write_image_data(rgba)
            .map_err(|e| McpError::internal_error(format!("png write: {e}"), None))?;
    }
    Ok(buf)
}

fn ok_json<T: Serialize>(v: &T) -> Result<CallToolResult, McpError> {
    let text = serde_json::to_string(v).map_err(|e| McpError::internal_error(format!("serialize: {e}"), None))?;
    Ok(CallToolResult::success(vec![Content::text(text)]))
}
