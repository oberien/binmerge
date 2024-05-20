use std::fmt::Write;
use std::ops::Range;
use crossterm::event::{KeyCode, KeyEvent};
use positioned_io::{RandomAccessFile, ReadAt};
use ratatui::buffer::Buffer;
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::prelude::{Line, Span, Stylize, Text};
use ratatui::symbols::border;
use ratatui::widgets::{Block, Borders, Paragraph, Widget};
use ratatui::widgets::block::Title;
use binmerge::range_tree::RangeTree;
use crate::AppCtx;
use crate::layers::{Layer, LayerCtx};

pub struct DiffView {
    file1: FileView,
    file2: FileView,
}
impl DiffView {
    pub fn new(file1: FileView, file2: FileView) -> DiffView {
        DiffView { file1, file2 }
    }
}
impl Layer<AppCtx> for DiffView {
    fn handle_key_event(&mut self, ctx: &mut LayerCtx<AppCtx>, evt: KeyEvent) {
        let app = ctx.ctx();
        match evt.code {
            KeyCode::Char('q') => app.exit = true,
            KeyCode::Down => app.increase_pos(16),
            KeyCode::Up => app.decrease_pos(16),
            KeyCode::PageDown => app.increase_pos(app.shown_data_height as u64 * 16),
            KeyCode::PageUp => app.decrease_pos(app.shown_data_height as u64 * 16),
            KeyCode::Char('N') => app.prev_diff(),
            KeyCode::Char('n') => app.next_diff(),
            KeyCode::Char('>') => if let Some(index) = app.current_diff_index {
                app.merges_1_into_2.insert(app.diffs.get(index).unwrap().clone());
                app.merges_2_into_1.remove_range_exact(app.diffs.get(index).unwrap().clone());
                app.leave_unmerged.remove_range_exact(app.diffs.get(index).unwrap().clone());
            }
            KeyCode::Char('<') => if let Some(index) = app.current_diff_index {
                app.merges_1_into_2.remove_range_exact(app.diffs.get(index).unwrap().clone());
                app.merges_2_into_1.insert(app.diffs.get(index).unwrap().clone());
                app.leave_unmerged.remove_range_exact(app.diffs.get(index).unwrap().clone());
            }
            KeyCode::Char('=') => if let Some(index) = app.current_diff_index {
                app.merges_1_into_2.remove_range_exact(app.diffs.get(index).unwrap().clone());
                app.merges_2_into_1.remove_range_exact(app.diffs.get(index).unwrap().clone());
                app.leave_unmerged.insert(app.diffs.get(index).unwrap().clone());
            }
            KeyCode::Char('!') => if let Some(index) = app.current_diff_index {
                app.merges_1_into_2.remove_range_exact(app.diffs.get(index).unwrap().clone());
                app.merges_2_into_1.remove_range_exact(app.diffs.get(index).unwrap().clone());
                app.leave_unmerged.remove_range_exact(app.diffs.get(index).unwrap().clone());
            }
            _ => (),
        }
    }

    fn render(&mut self, ctx: &mut LayerCtx<AppCtx>, area: Rect, buf: &mut Buffer) {
        let app = ctx.ctx();

        const HEX_PART_LEN: usize = 1 + 8*3 + 1 + 8*3 + 1;
        const ASCII_LEN: usize = 1 + 8 + 1 + 8 + 1;
        const WIDTH_PER_FILE: u16 = 1 + HEX_PART_LEN as u16 + ASCII_LEN as u16 + 1;
        //      + /foo/bar -----------------------------------------------------------++ baz +
        // 1330 | XX XX XX XX XX XX XX XX  XX XX XX XX XX XX XX XX  12345678 90abcdef || ... |
        // 1340 | ...                                                                 || ... |
        //      +---------------------------------------------------------------------++-----+
        // < overwrite left with right  > overwrite right with left  q quit
        let position_len = app.len.ilog(16) as usize + 1;

        let all = Layout::new(Direction::Vertical, [
            Constraint::Min(1),
            Constraint::Length(1),
            Constraint::Length(1),
        ]).split(area);
        let files = Layout::new(Direction::Horizontal, [
            Constraint::Length(position_len as u16),
            Constraint::Length(1),
            Constraint::Length(WIDTH_PER_FILE),
            Constraint::Length(1),
            Constraint::Length(WIDTH_PER_FILE),
        ]).split(all[0]);

        let positions = files[0];
        let left = files[2];
        let right = files[4];
        let instructions = all[1];
        let status_line = all[2];

        let mut content = String::with_capacity(positions.height as usize * position_len);
        content.push('\n');
        for i in 0..positions.height-2 {
            content.write_fmt(format_args!("{: >position_len$x}\n", app.pos + i as u64 * 16)).unwrap();
        }
        Paragraph::new(content).block(Block::new()).render(positions, buf);

        assert_eq!(left.height, right.height);
        app.shown_data_height = left.height - 2;
        let current_diff_range = app.current_diff_index
            .and_then(|i| app.diffs.get(i))
            .cloned()
            .unwrap_or(0..0);
        self.file1.render(
            left, buf, app.pos, current_diff_range.clone(),
            &app.diffs, &app.merges_2_into_1, &app.merges_1_into_2, &app.leave_unmerged,
        );
        self.file2.render(
            right, buf, app.pos, current_diff_range.clone(),
            &app.diffs, &app.merges_1_into_2, &app.merges_2_into_1, &app.leave_unmerged,
        );

        // instructions
        Line::from(vec![
            " <".blue().bold(),
            " overwrite left".into(),
            "  >".blue().bold(),
            " overwrite right".into(),
            "  =".blue().bold(),
            " leave unmerged".into(),
            "  !".blue().bold(),
            " reset this merge".into(),
            "  n/N".blue().bold(),
            " next/prev item".into(),
            // "  m/M".blue().bold(),
            // " next/prev merge".into(),
            // "  d/D".blue().bold(),
            // " next/prev diff".into(),
            "  q".blue().bold(),
            " quit ".into(),
        ]).centered().render(instructions, buf);

        // status
        let question_mark = app.all_diffs_loaded.then_some("").unwrap_or("?");
        Line::from(vec![
            {
                let diff = match app.current_diff_index {
                    Some(index) => format!("diff {}", index + 1),
                    None => "no diff ".to_string(),
                };
                format!("Looking at {diff}/{}{}   ", app.diffs.len(), question_mark)
            }.into(),
            format!(
                "Merged {}/{}{}   ",
                app.merges_1_into_2.len() + app.merges_2_into_1.len() + app.leave_unmerged.len(),
                app.diffs.len(),
                question_mark,
            ).into(),
            if app.all_diffs_loaded {
                format!("Found {} diffs", app.diffs.len())
            } else {
                format!("Loading diffs, {} so far", app.diffs.len())
            }.into(),
        ]).render(status_line, buf);
    }
}

pub struct FileView {
    name: String,
    file: RandomAccessFile,
}
impl FileView {
    pub fn new(name: String, file: RandomAccessFile) -> FileView {
        FileView { name, file }
    }
    fn render(
        &self, area: Rect, buf: &mut Buffer, pos: u64, current_diff_range: Range<u64>,
        diffs: &RangeTree<u64>, merged_into_this: &RangeTree<u64>, merged_from_this: &RangeTree<u64>,
        leave_unmerged: &RangeTree<u64>,
    ) {
        let len = (area.height as usize - 2) * 16;
        let mut data = vec![0u8; len];
        self.file.read_exact_at(pos, &mut data).unwrap();

        let mut hex_text = Text::default();
        let mut ascii_text = Text::default();
        for (line_index, chunk) in data.chunks(16).enumerate() {
            let mut hex_line = Line::default();
            let mut ascii_line = Line::default();

            for (i, byte) in chunk.iter().copied().enumerate() {
                let pos = pos + line_index as u64 * 16 + i as u64;
                let mut hex_span = Span::from(format!("{byte:02x} "));
                let mut ascii_span = if byte.is_ascii() && char::from(byte).escape_default().len() == 1 {
                    Span::from((byte as char).to_string())
                } else {
                    Span::from(".")
                };
                if merged_into_this.contains(pos) {
                    hex_span = hex_span.yellow().bold();
                    ascii_span = ascii_span.yellow().bold();
                } else if merged_from_this.contains(pos) {
                    hex_span = hex_span.green().bold();
                    ascii_span = ascii_span.green().bold();
                } else if leave_unmerged.contains(pos) {
                    hex_span = hex_span.light_green().bold();
                    ascii_span = ascii_span.light_green().bold();
                } else if diffs.contains(pos) {
                    hex_span = hex_span.red().bold();
                    ascii_span = ascii_span.red().bold();
                }
                if current_diff_range.contains(&pos) {
                    hex_span = hex_span.on_white();
                    ascii_span = ascii_span.on_white();
                }
                hex_line.push_span(hex_span);
                ascii_line.push_span(ascii_span);

                // separator space between first 8 and second 8 bytes
                if i == 7 {
                    hex_line.push_span(" ");
                    ascii_line.push_span(" ");
                }
            }
            hex_text.push_line(hex_line);
            ascii_text.push_line(ascii_line);
        }

        let title = Title::from(format!(" {} ", self.name).bold());
        let block = Block::default()
            .title(title.alignment(Alignment::Left))
            .borders(Borders::ALL)
            .border_set(border::THICK);
        let inner = block.inner(area);

        let layout = Layout::new(Direction::Horizontal, vec![
            Constraint::Length(1),
            Constraint::Length(8*3 + 1 + 8*3 - 1),
            Constraint::Length(2),
            Constraint::Length(8 + 1 + 8),
            Constraint::Length(1),
        ]).split(inner);
        let hex = layout[1];
        let ascii = layout[3];

        block.render(area, buf);
        Paragraph::new(hex_text).render(hex, buf);
        Paragraph::new(ascii_text).render(ascii, buf);
    }
}
