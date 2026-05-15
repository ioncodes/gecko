use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use backend_wgpu::GxRenderer;
use gecko::flipper::gx::draw::TextureFormat;
use gecko::host::{DrawVertex, GxAction, RenderSink, TextureKey};

pub struct TextureRecord {
    pub width: u32,
    pub height: u32,
    pub format: TextureFormat,
    pub rgba: Vec<u8>,
    pub last_seen_frame: u64,
}

#[derive(Default)]
pub struct Introspection {
    pub textures: HashMap<TextureKey, TextureRecord>,
    pub bound: [Option<TextureKey>; 8],
    pub last_xfb_size: (u32, u32),
    pub draw_count_this_frame: u64,
    pub frame_index: u64,
}

pub struct McpSink {
    pub gx: Arc<Mutex<GxRenderer>>,
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    pub introspect: Arc<Mutex<Introspection>>,
    /// Side scratch for vertices emitted by gecko since the trait method
    /// can't return `&mut` into the `Mutex<GxRenderer>`. Synced into the
    /// underlying renderer's `scratch_vertices` on every `exec`.
    pub scratch: Vec<DrawVertex>,
}

impl RenderSink for McpSink {
    fn exec(&mut self, action: GxAction) {
        match &action {
            GxAction::LoadTexture {
                id,
                width,
                height,
                fmt,
                rgba,
            } => {
                let mut i = self.introspect.lock().unwrap();
                let frame = i.frame_index;
                i.textures.insert(
                    *id,
                    TextureRecord {
                        width: *width,
                        height: *height,
                        format: *fmt,
                        rgba: rgba.clone(),
                        last_seen_frame: frame,
                    },
                );
            }
            GxAction::SetTexture { slot, id, .. } => {
                let mut i = self.introspect.lock().unwrap();
                if *slot < 8 {
                    i.bound[*slot] = Some(*id);
                }
            }
            GxAction::PresentXfb { width, height, .. } => {
                let mut i = self.introspect.lock().unwrap();
                i.last_xfb_size = (*width, *height);
                i.frame_index = i.frame_index.wrapping_add(1);
                i.draw_count_this_frame = 0;
            }
            GxAction::Draw(_) => {
                self.introspect.lock().unwrap().draw_count_this_frame += 1;
            }
            _ => {}
        }
        self.gx.lock().unwrap().process_action_with_external_scratch(
            &self.device,
            &self.queue,
            &action,
            &mut self.scratch,
        );
    }

    fn vertex_scratch(&mut self) -> &mut Vec<DrawVertex> {
        &mut self.scratch
    }
}
