use crossterm::event::KeyEvent;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::widgets::Widget;

pub struct Layers<Ctx> {
    layers: Vec<Box<dyn Layer<Ctx>>>,
    ctx: Ctx,
}
pub struct LayerChanges<Ctx> {
    layer_changes: Vec<LayerChange<Ctx>>,
}
enum LayerChange<Ctx> {
    Push(Box<dyn Layer<Ctx>>),
    Pop,
}

pub trait Layer<Ctx> {
    fn handle_key_event(&mut self, ctx: &mut Ctx, layers: &mut LayerChanges<Ctx>, evt: KeyEvent);
    fn render(&mut self, ctx: &mut Ctx, layers: &mut LayerChanges<Ctx>, area: Rect, buf: &mut Buffer);
}

impl<Ctx> Layers<Ctx> {
    pub fn new(ctx: Ctx) -> Layers<Ctx> {
        Layers { layers: Vec::new(), ctx }
    }
    pub fn handle_key_event(&mut self, evt: KeyEvent) {
        let Some(last) = self.layers.last_mut() else { return };
        let mut layer_changes = LayerChanges { layer_changes: Vec::new() };
        last.handle_key_event(&mut self.ctx, &mut layer_changes, evt);
        self.apply_layer_changes(layer_changes);
    }
    pub fn ctx(&mut self) -> &mut Ctx {
        &mut self.ctx
    }
    pub fn push_layer(&mut self, layer: impl Layer<Ctx> + 'static) {
        self.layers.push(Box::new(layer));
    }
    pub fn pop_layer(&mut self) {
        self.layers.pop();
    }
    fn apply_layer_changes(&mut self, changes: LayerChanges<Ctx>) {
        for change in changes.layer_changes {
            match change {
                LayerChange::Push(layer) => self.layers.push(layer),
                LayerChange::Pop => drop(self.layers.pop()),
            }
        }
    }
}

impl<Ctx> LayerChanges<Ctx> {
    pub fn push_layer(&mut self, layer: impl Layer<Ctx> + 'static) {
        self.layer_changes.push(LayerChange::Push(Box::new(layer)));
    }
    pub fn pop_layer(&mut self) {
        self.layer_changes.push(LayerChange::Pop);
    }
}

impl<Ctx> Widget for &mut Layers<Ctx> {
    fn render(self, area: Rect, buf: &mut Buffer) where Self: Sized {
        let mut layer_changes = LayerChanges { layer_changes: Vec::new() };
        for layer in &mut self.layers {
            layer.render(&mut self.ctx, &mut layer_changes, area, buf);
        }
        self.apply_layer_changes(layer_changes);
    }
}
