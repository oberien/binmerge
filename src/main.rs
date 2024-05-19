use std::{io, panic, thread};
use std::fmt::Write;
use std::fs::{File, OpenOptions};
use std::io::{Seek, SeekFrom, Stdout};
use std::ops::Range;
use std::path::{Path, PathBuf};

use clap::Parser;
use crossbeam_channel::{Receiver, Select};
use crossterm::event;
use crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind};
use crossterm::terminal::{EnterAlternateScreen, LeaveAlternateScreen};
use positioned_io::{RandomAccessFile, ReadAt};
use ratatui::backend::CrosstermBackend;
use ratatui::buffer::Buffer;
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::Stylize;
use ratatui::symbols::border;
use ratatui::Terminal;
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Borders, Paragraph, Widget};
use ratatui::widgets::block::Title;

use binmerge::range_tree::RangeTree;

use crate::diff_iter::DiffIter;

mod diff_iter;

#[derive(clap::Parser)]
struct Args {
    file1: PathBuf,
    file2: PathBuf,
}

fn main() {
    let args = Args::parse();
    let mut app = App::new(args);

    // setup panic hooks
    let hook = panic::take_hook();
    panic::set_hook(Box::new(move |panic_info| {
        restore_terminal();
        hook(panic_info);
    }));

    // init ratatui
    crossterm::execute!(io::stdout(), EnterAlternateScreen).unwrap();
    crossterm::terminal::enable_raw_mode().unwrap();
    let mut terminal = Terminal::new(CrosstermBackend::new(io::stdout())).unwrap();

    app.run(&mut terminal);

    // reset terminal
    restore_terminal();
}

fn restore_terminal() {
    crossterm::execute!(io::stdout(), LeaveAlternateScreen).unwrap();
    crossterm::terminal::disable_raw_mode().unwrap();
}

pub type Tui = Terminal<CrosstermBackend<Stdout>>;
struct App {
    name1: String,
    name2: String,
    exit: bool,
    shown_data_height: u16,
    pos: u64,
    len: u64,
    diffs: RangeTree<u64>,
    all_diffs_loaded: bool,
    file1: RandomAccessFile,
    file2: RandomAccessFile,
    diff_rx: Option<Receiver<Range<u64>>>,
    event_rx: Receiver<Event>,
}
impl App {
    fn new(args: Args) -> App {
        fn open_write(path: impl AsRef<Path>) -> File {
            OpenOptions::new().create(false).read(true).write(true).append(false)
                .open(path).unwrap()
        }
        // _Technically_ there is a TOCTOU if the files get exchanged between first and second open,
        // but there's no easy way to fix it.
        // Windows has ReOpenFile to get a new handle with a separate cursor
        // Linux needs to use pread / pwrite to not disturb the cursor
        let mut a = open_write(&args.file1);
        let a2 = File::open(&args.file1).unwrap();
        let mut b = open_write(&args.file2);
        let b2 = File::open(&args.file2).unwrap();
        // we can't use metadata on block devices, so use seek instead
        let alen = a.seek(SeekFrom::End(0)).unwrap();
        a.seek(SeekFrom::Start(0)).unwrap();
        let blen = b.seek(SeekFrom::End(0)).unwrap();
        b.seek(SeekFrom::Start(0)).unwrap();
        assert_eq!(alen, blen, "files have different lengths");

        // diff thread
        let (diff_tx, diff_rx) = crossbeam_channel::unbounded();
        thread::spawn(move || {
            let diff_iter = DiffIter::new(a2, b2);
            for part in diff_iter {
                diff_tx.send(part).unwrap();
            }
        });

        // event thread
        let (event_tx, event_rx) = crossbeam_channel::bounded(0);
        thread::spawn(move || {
            loop {
                event_tx.send(event::read().unwrap()).unwrap();
            }
        });

        App {
            name1: args.file1.to_string_lossy().into_owned(),
            name2: args.file2.to_string_lossy().into_owned(),
            exit: false,
            shown_data_height: 0,
            pos: 0,
            len: alen,
            diffs: RangeTree::new(),
            all_diffs_loaded: false,
            file1: RandomAccessFile::try_new(a).unwrap(),
            file2: RandomAccessFile::try_new(b).unwrap(),
            diff_rx: Some(diff_rx),
            event_rx,
        }
    }

    pub fn run(&mut self, terminal: &mut Tui) {
        while !self.exit {
            terminal.draw(|frame| frame.render_widget(&mut *self, frame.size())).unwrap();
            let mut sel = Select::new();
            let diff_rx_index = self.diff_rx.as_ref()
                .map(|diff_rx| sel.recv(diff_rx));
            let event_rx = sel.recv(&self.event_rx);
            let op = sel.select();
            match op.index() {
                i if Some(i) == diff_rx_index => match op.recv(self.diff_rx.as_ref().unwrap()) {
                    Ok(diff) => self.diffs.append(diff),
                    Err(_) => {
                        self.all_diffs_loaded = true;
                        self.diff_rx.take();
                    }
                }
                i if i == event_rx => match op.recv(&self.event_rx).unwrap() {
                    Event::Key(key_event) if key_event.kind == KeyEventKind::Press => {
                        self.handle_key_event(key_event)
                    }
                    _ => {}
                }
                _ => unreachable!(),
            }
        }
    }

    fn handle_key_event(&mut self, evt: KeyEvent) {
        match evt.code {
            KeyCode::Char('q') => self.exit = true,
            KeyCode::Down => self.increase_pos(16),
            KeyCode::Up => self.decrease_pos(16),
            KeyCode::PageDown => self.increase_pos(self.shown_data_height as u64 * 16),
            KeyCode::PageUp => self.decrease_pos(self.shown_data_height as u64 * 16),
            _ => (),
        }
    }

    fn decrease_pos(&mut self, by: u64) {
        self.pos = self.pos.saturating_sub(by);
        assert_eq!(self.pos % 16, 0);
    }
    fn increase_pos(&mut self, by: u64) {
        self.pos += by;
        let bytes_shown = self.shown_data_height as u64 * 16;
        let max_pos = self.len - bytes_shown;
        let max_pos = max_pos - (max_pos % 16) + 16;
        self.pos = self.pos.min(max_pos);
        assert_eq!(self.pos % 16, 0);
    }
}

impl Widget for &mut App {
    fn render(self, area: Rect, buf: &mut Buffer) {
        //      + /foo/bar ----------------------------------------------------------++ baz +
        // 1330 | XX XX XX XX XX XX XX XX  XX XX XX XX XX XX XX XX  1234567890abcdef || ... |
        // 1340 | ...                                                                || ... |
        //      +--------------------------------------------------------------------++-----+
        // < overwrite left with right  > overwrite right with left  q quit
        const HEX_PART_LEN: usize = 1 + 8*3 + 1 + 8*3 + 1;
        const ASCII_LEN: usize = 16 + 1;
        const WIDTH_PER_FILE: u16 = 1 + HEX_PART_LEN as u16 + ASCII_LEN as u16 + 1;
        let position_len = self.len.ilog(16) as usize + 1;

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

        let render_file = |name: &str, file: &RandomAccessFile, height: u16| {
            let len = height as usize * 16;
            let mut data = vec![0u8; len];
            file.read_exact_at(self.pos, &mut data).unwrap();

            let mut text = Text::default();
            for (line_index, chunk) in data.chunks(16).enumerate() {
                let mut line = Line::default();

                // write hex
                line.push_span(" ");
                let mut written = 1;
                for (i, byte) in chunk.iter().enumerate() {
                    let mut span = Span::from(format!("{byte:02x} "));
                    if self.diffs.contains(self.pos + line_index as u64 * 16 + i as u64) {
                        span = span.red();
                    }
                    line.push_span(span);
                    written += 3;
                    if i == 7 {
                        line.push_span(" ");
                        written += 1;
                    }
                }
                // fill with spaces until ascii part (also handles non-complete chunks)
                for _ in written..HEX_PART_LEN {
                    line.push_span(" ");
                }

                // write ascii
                for (i, &byte) in chunk.iter().enumerate() {
                    let mut span = if byte.is_ascii() && char::from(byte).escape_default().len() == 1 {
                        Span::from((byte as char).to_string())
                    } else {
                        Span::from(".")
                    };
                    if self.diffs.contains(self.pos + line_index as u64 * 16 + i as u64) {
                        span = span.red();
                    }
                    line.push_span(span);
                }

                text.push_line(line);
            }

            let title = Title::from(format!(" {name} ").bold());
            let block = Block::default()
                .title(title.alignment(Alignment::Left))
                .borders(Borders::ALL)
                .border_set(border::THICK);
            Paragraph::new(text).block(block)
        };

        let mut content = String::with_capacity(positions.height as usize * position_len);
        content.push('\n');
        for i in 0..positions.height-2 {
            content.write_fmt(format_args!("{: >position_len$x}\n", self.pos + i as u64 * 16)).unwrap();
        }
        Paragraph::new(content).block(Block::new()).render(positions, buf);

        assert_eq!(left.height, right.height);
        self.shown_data_height = left.height - 2;
        render_file(&self.name1, &self.file1, left.height - 2).render(left, buf);
        render_file(&self.name2, &self.file2, right.height - 2).render(right, buf);

        // instructions
        Line::from(vec![
            " <".blue().bold(),
            " overwrite left with right".into(),
            "  >".blue().bold(),
            " overwrite right with left".into(),
            "  q".blue().bold(),
            " quit ".into(),
        ]).centered().render(instructions, buf);

        // status
        Line::from(
            if self.all_diffs_loaded {
                format!("Found {} diffs", self.diffs.len())
            } else {
                format!("Loading diffs, {} so far", self.diffs.len())
            }
        ).render(status_line, buf);
    }
}
