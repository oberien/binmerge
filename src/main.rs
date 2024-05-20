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

use binmerge::diff_iter::DiffIter;
use binmerge::range_tree::RangeTree;

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
    exit: bool,
    shown_data_height: u16,
    pos: u64,
    len: u64,
    diffs: RangeTree<u64>,
    current_diff_index: Option<usize>,
    all_diffs_loaded: bool,
    file1: FileView,
    file2: FileView,
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
            exit: false,
            shown_data_height: 0,
            pos: 0,
            len: alen,
            diffs: RangeTree::new(),
            current_diff_index: None,
            all_diffs_loaded: false,
            file1: FileView::new(args.file1.to_string_lossy().into_owned(), RandomAccessFile::try_new(a).unwrap()),
            file2: FileView::new(args.file2.to_string_lossy().into_owned(), RandomAccessFile::try_new(b).unwrap()),
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
            KeyCode::Char('N') => self.prev_diff(),
            KeyCode::Char('n') => self.next_diff(),
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

    fn prev_diff(&mut self) {
        self.current_diff_index = Some(match self.current_diff_index {
            None | Some(0) => self.diffs.len().saturating_sub(1),
            Some(index) => index - 1,
        });
        self.center_diff();
    }
    fn next_diff(&mut self) {
        self.current_diff_index = Some(match self.current_diff_index {
            Some(index) => (index + 1) % self.diffs.len(),
            None => 0,
        });
        self.center_diff();
    }
    fn center_diff(&mut self) {
        let range = match self.current_diff_index.and_then(|i| self.diffs.get(i)) {
            Some(range) => range,
            None => return,
        };
        let len = range.end - range.start;
        let bytes_shown = self.shown_data_height as u64 * 16;
        if len > bytes_shown - 48 {
            self.pos = range.start.saturating_sub(32);
        } else {
            let top_offset = (bytes_shown - len) / 2;
            self.pos = range.start.saturating_sub(top_offset);
        }

        self.pos -= self.pos % 16;
        assert_eq!(self.pos % 16, 0);
    }
}

const HEX_PART_LEN: usize = 1 + 8*3 + 1 + 8*3 + 1;
const ASCII_LEN: usize = 8 + 1 + 8 + 1;
const WIDTH_PER_FILE: u16 = 1 + HEX_PART_LEN as u16 + ASCII_LEN as u16 + 1;

impl Widget for &mut App {
    fn render(self, area: Rect, buf: &mut Buffer) {
        //      + /foo/bar -----------------------------------------------------------++ baz +
        // 1330 | XX XX XX XX XX XX XX XX  XX XX XX XX XX XX XX XX  12345678 90abcdef || ... |
        // 1340 | ...                                                                 || ... |
        //      +---------------------------------------------------------------------++-----+
        // < overwrite left with right  > overwrite right with left  q quit
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

        let mut content = String::with_capacity(positions.height as usize * position_len);
        content.push('\n');
        for i in 0..positions.height-2 {
            content.write_fmt(format_args!("{: >position_len$x}\n", self.pos + i as u64 * 16)).unwrap();
        }
        Paragraph::new(content).block(Block::new()).render(positions, buf);

        assert_eq!(left.height, right.height);
        self.shown_data_height = left.height - 2;
        let current_diff_range = self.current_diff_index
            .and_then(|i| self.diffs.get(i))
            .cloned()
            .unwrap_or(0..0);
        self.file1.render(left, buf, self.pos, current_diff_range.clone(), &self.diffs);
        self.file2.render(right, buf, self.pos, current_diff_range.clone(), &self.diffs);

        // instructions
        Line::from(vec![
            " <".blue().bold(),
            " overwrite left with right".into(),
            "  >".blue().bold(),
            " overwrite right with left".into(),
            "  n".blue().bold(),
            " next".into(),
            "  N".blue().bold(),
            " prev".into(),
            "  q".blue().bold(),
            " quit ".into(),
        ]).centered().render(instructions, buf);

        // status
        Line::from(vec![
            match self.current_diff_index {
                Some(index) => format!("Looking at diff {} / {}{}   ", index + 1, self.diffs.len(), match self.all_diffs_loaded {
                    true => "",
                    false => "?",
                }),
                None => "".to_owned(),
            }.into(),
            if self.all_diffs_loaded {
                format!("Found {} diffs", self.diffs.len())
            } else {
                format!("Loading diffs, {} so far", self.diffs.len())
            }.into(),
        ]).render(status_line, buf);
    }
}

struct FileView {
    name: String,
    file: RandomAccessFile,
}
impl FileView {
    fn new(name: String, file: RandomAccessFile) -> FileView {
        FileView { name, file }
    }
    fn render(&self, area: Rect, buf: &mut Buffer, pos: u64, current_diff_range: Range<u64>, diffs: &RangeTree<u64>) where Self: Sized {
        let len = (area.height as usize - 2) * 16;
        let mut data = vec![0u8; len];
        self.file.read_exact_at(pos, &mut data).unwrap();

        let mut text = Text::default();
        for (line_index, chunk) in data.chunks(16).enumerate() {
            let mut line = Line::default();

            // write hex
            line.push_span(" ");
            let mut written = 1;
            for (i, byte) in chunk.iter().enumerate() {
                let mut span = Span::from(format!("{byte:02x} "));
                if diffs.contains(pos + line_index as u64 * 16 + i as u64) {
                    span = span.red().bold();
                }
                if current_diff_range.contains(&(pos + line_index as u64 * 16 + i as u64)) {
                    span = span.on_white();
                }
                line.push_span(span);
                written += 3;

                // separator space between first 8 and second 8 hex numbers
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
                if diffs.contains(pos + line_index as u64 * 16 + i as u64) {
                    span = span.red().bold();
                }
                if current_diff_range.contains(&(pos + line_index as u64 * 16 + i as u64)) {
                    span = span.on_white();
                }
                line.push_span(span);

                // separator space between first 8 and second 8 hex numbers
                if i == 7 {
                    line.push_span(" ");
                    written += 1;
                }
            }

            text.push_line(line);
        }

        let title = Title::from(format!(" {} ", self.name).bold());
        let block = Block::default()
            .title(title.alignment(Alignment::Left))
            .borders(Borders::ALL)
            .border_set(border::THICK);
        Paragraph::new(text).block(block).render(area, buf);
    }
}
