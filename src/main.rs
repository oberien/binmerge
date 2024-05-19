use std::{io, mem, panic};
use std::fmt::Write;
use std::fs::{File, OpenOptions};
use std::io::{BufReader, Bytes, Read, Seek, SeekFrom, Stdout};
use std::iter::Zip;
use std::path::{Path, PathBuf};

use clap::Parser;
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
use ratatui::text::Line;
use ratatui::widgets::{Block, Borders, Paragraph, Widget};
use ratatui::widgets::block::Title;

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
    last_data_height: u16,
    diff_iter: DiffIter,
    pos: u64,
    len: u64,
    file1: RandomAccessFile,
    file2: RandomAccessFile,
}
impl App {
    fn new(args: Args) -> App {
        fn open_write(path: impl AsRef<Path>) -> File {
            OpenOptions::new().create(false).read(true).write(true).append(false)
                .open(path).unwrap()
        }
        // _technically_ there is a TOCTOU if the files get exchanged between both openings,
        // but there's no easy way to fix it
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

        App {
            name1: args.file1.to_string_lossy().into_owned(),
            name2: args.file2.to_string_lossy().into_owned(),
            exit: false,
            last_data_height: 0,
            diff_iter: DiffIter::new(a2, b2),
            pos: 0,
            len: alen,
            file1: RandomAccessFile::try_new(a).unwrap(),
            file2: RandomAccessFile::try_new(b).unwrap(),
        }
    }

    pub fn run(&mut self, terminal: &mut Tui) {
        while !self.exit {
            terminal.draw(|frame| frame.render_widget(&mut *self, frame.size())).unwrap();
            match event::read().unwrap() {
                Event::Key(key_event) if key_event.kind == KeyEventKind::Press => {
                    self.handle_key_event(key_event)
                }
                _ => {}
            };
        }
    }

    fn decrease_pos(&mut self, by: u64) {
        self.pos = self.pos.saturating_sub(by);
    }
    fn increase_pos(&mut self, by: u64) {
        self.pos += by;
        if self.pos >= self.len {
            self.pos = (self.len % 16).saturating_sub(16);
        }
    }

    fn handle_key_event(&mut self, evt: KeyEvent) {
        match evt.code {
            KeyCode::Char('q') => self.exit = true,
            KeyCode::Down => self.increase_pos(16),
            KeyCode::Up => self.decrease_pos(16),
            KeyCode::PageDown => self.increase_pos(self.last_data_height as u64 * 16),
            KeyCode::PageUp => self.decrease_pos(self.last_data_height as u64 * 16),
            _ => (),
        }
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

            let mut content = String::with_capacity((WIDTH_PER_FILE as usize - 2) * height as usize + height as usize);
            for chunk in data.chunks(16) {
                // write hex
                content.push(' ');
                for (i, byte) in chunk.iter().enumerate() {
                    content.write_fmt(format_args!("{byte:02x}")).unwrap();
                    content.push(' ');
                    if i == 7 {
                        content.push(' ');
                    }
                }
                // fill with spaces until ascii part (also handles non-complete chunks)
                for _ in content.len()..HEX_PART_LEN {
                    content.push(' ');
                }

                // write ascii
                for &byte in chunk {
                    if byte.is_ascii() && char::from(byte).escape_default().len() == 1 {
                        content.push(byte as char);
                    } else {
                        content.push('.');
                    }
                }

                content.push('\n');
            }

            let title = Title::from(format!(" {name} ").bold());
            let block = Block::default()
                .title(title.alignment(Alignment::Left))
                .borders(Borders::ALL)
                .border_set(border::THICK);
            Paragraph::new(content).block(block)
        };

        let mut content = String::with_capacity(positions.height as usize * position_len + position_len);
        content.push('\n');
        for i in 0..positions.height-2 {
            content.write_fmt(format_args!("{: >position_len$x}\n", self.pos + i as u64 * 16)).unwrap();
        }
        Paragraph::new(content).block(Block::new()).render(positions, buf);

        assert_eq!(left.height, right.height);
        self.last_data_height = left.height - 2;
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
    }
}

#[derive(Debug, Copy, Clone)]
enum Part {
    /// start, length
    Equal(u64, u64),
    /// start, length
    Different(u64, u64),
}
struct DiffIter {
    iter: Zip<Bytes<BufReader<File>>, Bytes<BufReader<File>>>,
    state: Part,
}
impl DiffIter {
    fn new(a: File, b: File) -> DiffIter {
        let mut a = BufReader::with_capacity(8*1024*1024, a);
        let mut b = BufReader::with_capacity(8*1024*1024, b);
        DiffIter {
            iter: a.bytes().zip(b.bytes()),
            state: Part::Equal(0, 0),
        }
    }
}

impl Iterator for DiffIter {
    type Item = Part;

    fn next(&mut self) -> Option<Self::Item> {
        while let Some((a, b)) = self.iter.next() {
            let a = a.unwrap();
            let b = b.unwrap();
            self.state = match (a == b, self.state) {
                (true, Part::Equal(start, len)) => Part::Equal(start, len + 1),
                (true, Part::Different(start, len)) => {
                    self.state = Part::Equal(start + len, 1);
                    return Some(Part::Different(start, len))
                }
                (false, Part::Equal(start, len)) => {
                    self.state = Part::Different(start + len, 1);
                    return Some(Part::Equal(start, len))
                }
                (false, Part::Different(start, len)) => Part::Different(start, len + 1),
            }
        }
        if let Part::Equal(0, 0) = self.state {
            None
        } else {
            Some(mem::replace(&mut self.state, Part::Equal(0, 0)))
        }
    }
}
