use std::sync::Arc;

use iced::advanced::graphics::Viewport;
use iced::keyboard::key::Code;
use iced::widget::shader::{self, Pipeline, Primitive, Program, Shader};
use iced::{Rectangle, mouse, window};

use crate::app::Message;
use crate::player::state::{self, PlayerState};

pub fn shader_widget(state: Arc<PlayerState>, window: window::Id) -> Shader<Message, PlayerProgram> {
    Shader::new(PlayerProgram { state, window })
        .width(iced::Length::Fill)
        .height(iced::Length::Fill)
}

pub struct PlayerProgram {
    state: Arc<PlayerState>,
    window: window::Id,
}

impl Program<Message> for PlayerProgram {
    type State = ();
    type Primitive = PlayerPrimitive;

    fn draw(&self, _state: &Self::State, _cursor: mouse::Cursor, _bounds: Rectangle) -> Self::Primitive {
        PlayerPrimitive {
            state: self.state.clone(),
        }
    }

    fn update(
        &self,
        _state: &mut Self::State,
        event: &iced::Event,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> Option<shader::Action<Message>> {
        match event {
            iced::Event::Keyboard(kbd) => match kbd {
                iced::keyboard::Event::KeyPressed {
                    physical_key, repeat, ..
                } => {
                    let hotkey = match state::physical_to_code(physical_key) {
                        Some(_) if *repeat => None,
                        Some(Code::Space) => Some(Message::PlayerTogglePause(self.window)),
                        Some(Code::Tab) => Some(Message::PlayerToggleUncapped(self.window)),
                        Some(Code::F10) => Some(Message::PlayerToggleFullscreen(self.window)),
                        Some(Code::F11) => Some(Message::PlayerToggleOverlay(self.window)),
                        Some(Code::F12) => Some(Message::PlayerScreenshot(self.window)),
                        Some(code) => {
                            self.state.handle_keyboard(code, true);
                            None
                        }
                        None => None,
                    };

                    match hotkey {
                        Some(message) => Some(shader::Action::publish(message)),
                        None => Some(shader::Action::request_redraw()),
                    }
                }
                iced::keyboard::Event::KeyReleased { physical_key, .. } => {
                    if let Some(code) = state::physical_to_code(physical_key) {
                        self.state.handle_keyboard(code, false);
                    }
                    Some(shader::Action::request_redraw())
                }
                _ => None,
            },
            iced::Event::Mouse(ev) => match ev {
                iced::mouse::Event::CursorMoved { .. } => {
                    if let Some(p) = cursor.position_in(bounds) {
                        let nx = p.x / bounds.width.max(1.0);
                        let ny = p.y / bounds.height.max(1.0);
                        self.state.set_ir_pointer(nx, ny);
                    } else {
                        self.state.clear_ir_pointer();
                    }
                    Some(shader::Action::request_redraw())
                }
                iced::mouse::Event::CursorLeft => {
                    self.state.clear_ir_pointer();
                    Some(shader::Action::request_redraw())
                }
                iced::mouse::Event::ButtonPressed(button) => {
                    if cursor.is_over(bounds) {
                        self.state.handle_mouse_button(*button, true);
                    }
                    Some(shader::Action::request_redraw())
                }
                iced::mouse::Event::ButtonReleased(button) => {
                    self.state.handle_mouse_button(*button, false);
                    Some(shader::Action::request_redraw())
                }
                _ => None,
            },
            _ => None,
        }
    }

    fn mouse_interaction(&self, _state: &Self::State, bounds: Rectangle, cursor: mouse::Cursor) -> mouse::Interaction {
        if cursor.is_over(bounds) {
            mouse::Interaction::Hidden
        } else {
            mouse::Interaction::None
        }
    }
}

#[derive(Debug)]
pub struct PlayerPrimitive {
    state: Arc<PlayerState>,
}

impl Primitive for PlayerPrimitive {
    type Pipeline = PlayerPipeline;

    fn prepare(
        &self,
        pipeline: &mut Self::Pipeline,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        _bounds: &Rectangle,
        _viewport: &Viewport,
    ) {
        PlayerState::start_boot(&self.state, device, queue, pipeline.format);
    }

    fn render(
        &self,
        _pipeline: &Self::Pipeline,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
        clip_bounds: &Rectangle<u32>,
    ) {
        self.state.blit(
            encoder,
            target,
            (clip_bounds.width.max(1), clip_bounds.height.max(1)),
            wgpu::LoadOp::Load,
        );
    }
}

#[derive(Debug)]
pub struct PlayerPipeline {
    format: wgpu::TextureFormat,
}

impl Pipeline for PlayerPipeline {
    fn new(_device: &wgpu::Device, _queue: &wgpu::Queue, format: wgpu::TextureFormat) -> Self {
        Self { format }
    }
}
