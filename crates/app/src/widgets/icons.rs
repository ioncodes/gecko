use iced::advanced::graphics::mesh::{self, Renderer as _};
use iced::advanced::widget::Tree;
use iced::advanced::{Layout, Widget, layout, mouse, renderer};
use iced::{Color, Element, Length, Rectangle, Size, Transformation};

pub fn chevron_right<Message: 'static>(color: Color) -> Element<'static, Message> {
    Element::new(ChevronRight(color))
}

struct ChevronRight(Color);

impl<Message, Theme> Widget<Message, Theme, iced::Renderer> for ChevronRight {
    fn size(&self) -> Size<Length> {
        Size::new(Length::Fixed(14.0), Length::Fixed(14.0))
    }

    fn layout(&mut self, _tree: &mut Tree, _renderer: &iced::Renderer, limits: &layout::Limits) -> layout::Node {
        layout::Node::new(limits.resolve(14.0, 14.0, Size::ZERO))
    }

    fn draw(
        &self,
        _tree: &Tree,
        renderer: &mut iced::Renderer,
        _theme: &Theme,
        _style: &renderer::Style,
        layout: Layout<'_>,
        _cursor: mouse::Cursor,
        viewport: &Rectangle,
    ) {
        let bounds = layout.bounds();
        let Some(clip_bounds) = bounds.intersection(viewport) else {
            return;
        };
        let color = iced::advanced::graphics::color::pack(self.0);
        let vertices = [[4.5, 2.5], [9.0, 7.0], [4.5, 11.5], [3.5, 10.5], [7.0, 7.0], [3.5, 3.5]]
            .into_iter()
            .map(|[x, y]| mesh::SolidVertex2D {
                position: [bounds.x + x, bounds.y + y],
                color,
            })
            .collect();
        renderer.draw_mesh(mesh::Mesh::Solid {
            buffers: mesh::Indexed {
                vertices,
                indices: vec![0, 1, 4, 0, 4, 5, 1, 2, 3, 1, 3, 4],
            },
            transformation: Transformation::IDENTITY,
            clip_bounds,
        });
    }
}
