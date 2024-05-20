use crossterm::event::{KeyCode, KeyEvent};
use ratatui::buffer::Buffer;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Style, Stylize};
use ratatui::text::Text;
use ratatui::widgets::{Block, Clear, Paragraph, Widget};
use ratatui::widgets::block::Title;
use crate::AppCtx;
use crate::layers::{Layer, LayerChanges};

pub struct PopupYesNo {
    title: Title<'static>,
    text: Text<'static>,
    yes_selected: bool,
}

impl PopupYesNo {
    pub fn new(title: impl Into<Title<'static>>, text: impl Into<Text<'static>>) -> PopupYesNo {
        PopupYesNo {
            title: title.into(),
            text: text.into(),
            yes_selected: false,
        }
    }
}

impl Layer<AppCtx> for PopupYesNo {
    fn handle_key_event(&mut self, _ctx: &mut AppCtx, layers: &mut LayerChanges<AppCtx>, evt: KeyEvent) {
        match evt.code {
            KeyCode::Left | KeyCode::Right => self.yes_selected = !self.yes_selected,
            KeyCode::Esc | KeyCode::Char('q') => layers.pop_layer(),
            KeyCode::Enter if !self.yes_selected => layers.pop_layer(),
            KeyCode::Enter if self.yes_selected => {
                todo!()
            }
            _ => (),
        }
    }

    fn render(&mut self, _ctx: &mut AppCtx, _layers: &mut LayerChanges<AppCtx>, area: Rect, buf: &mut Buffer) {
        let layout = Layout::vertical([
            Constraint::Fill(1),
            Constraint::Length(1 + self.text.height() as u16 + 2 + 1),
            Constraint::Fill(1),
        ]).split(area);
        let layout = Layout::horizontal([
            Constraint::Fill(1),
            // at least 12 for ` <YES> <NO> `
            Constraint::Length(1 + self.text.width().max(12) as u16 + 1),
            Constraint::Fill(1),
        ]).split(layout[1]);
        let area = layout[1];

        // clear out the background
        Clear.render(area, buf);
        let block = Block::bordered()
            .title(self.title.clone())
            .style(Style::default().bg(Color::DarkGray));

        // layout for the buttons
        let layout = Layout::vertical([
            Constraint::Min(0),
            Constraint::Length(1),
        ]).split(block.inner(area));
        let layout = Layout::horizontal([
            Constraint::Fill(1),
            Constraint::Length(5),
            Constraint::Fill(1),
            Constraint::Length(4),
            Constraint::Fill(1),
        ]).split(layout[1]);
        let yes_area = layout[1];
        let no_area = layout[3];
        Paragraph::new(self.text.clone())
            .block(block)
            .render(area, buf);
        let (yes, no) = match self.yes_selected {
            true => (
                "<YES>".on_light_red(),
                "<NO>".into(),
            ),
            false => (
                "<YES>".into(),
                "<NO>".on_light_red(),
            ),
        };
        yes.render(yes_area, buf);
        no.render(no_area, buf);
    }
}
