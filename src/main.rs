use std::{io, panic, thread};
use std::fs::{File, OpenOptions};
use std::io::{Seek, SeekFrom, Stdout};
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::time::Instant;

use clap::Parser;
use crossbeam_channel::{Receiver, Select};
use crossterm::{cursor, event};
use crossterm::event::{Event, KeyEventKind};
use crossterm::terminal::{EnterAlternateScreen, LeaveAlternateScreen};
use positioned_io::RandomAccessFile;
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;

use binmerge::diff_iter::{BytesDiffIter, MemchrDiffIter, ThreadedDiffIter};
use binmerge::range_tree::RangeTree;

use crate::diff_view::DiffView;
use crate::layers::Layers;

mod apply;
mod layers;
mod diff_view;
mod popup;

#[derive(clap::Parser)]
struct Args {
    #[clap(long)]
    bench: Option<Bench>,
    file1: PathBuf,
    file2: PathBuf,
}
#[derive(clap::ValueEnum, Copy, Clone)]
enum Bench {
    Bytes,
    Memchr,
    Threaded,
}

fn main() {
    let args = Args::parse();

    if args.bench.is_some() {
        bench(args);
        return;
    }

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
    crossterm::terminal::disable_raw_mode().unwrap();
    crossterm::execute!(io::stdout(), LeaveAlternateScreen, cursor::Show).unwrap();
}

struct AppCtx {
    name1: String,
    name2: String,
    file1: RandomAccessFile,
    file2: RandomAccessFile,
    exit: bool,
    shown_data_height: u16,
    pos: u64,
    len: u64,
    diffs: RangeTree<u64>,
    current_diff_index: Option<usize>,
    all_diffs_loaded: bool,
    merges_1_into_2: RangeTree<u64>,
    merges_2_into_1: RangeTree<u64>,
    leave_unmerged: RangeTree<u64>,
}

pub type Tui = Terminal<CrosstermBackend<Stdout>>;
struct App {
    diff_rx: Option<Receiver<Range<u64>>>,
    event_rx: Receiver<Event>,
    layers: Layers<AppCtx>,
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
            let diff_iter = ThreadedDiffIter::new(a2, b2);
            for diff in diff_iter {
                diff_tx.send(diff).unwrap();
            }
        });

        // event thread
        let (event_tx, event_rx) = crossbeam_channel::bounded(0);
        thread::spawn(move || {
            loop {
                event_tx.send(event::read().unwrap()).unwrap();
            }
        });


        let ctx = AppCtx {
            name1: args.file1.to_string_lossy().into_owned(),
            name2: args.file2.to_string_lossy().into_owned(),
            file1: RandomAccessFile::try_new(a).unwrap(),
            file2: RandomAccessFile::try_new(b).unwrap(),
            exit: false,
            shown_data_height: 0,
            pos: 0,
            len: alen,
            diffs: RangeTree::new(),
            current_diff_index: None,
            all_diffs_loaded: false,
            merges_1_into_2: RangeTree::new(),
            merges_2_into_1: RangeTree::new(),
            leave_unmerged: RangeTree::new(),
        };
        let diff_view = DiffView::new();
        let mut layers = Layers::new(ctx);
        layers.push_layer(diff_view);
        App {
            diff_rx: Some(diff_rx),
            event_rx,
            layers,
        }
    }

    pub fn run(&mut self, terminal: &mut Tui) {
        while !self.layers.ctx().exit {
            terminal.draw(|frame| frame.render_widget(&mut self.layers, frame.size())).unwrap();
            let mut sel = Select::new();
            let diff_rx_index = self.diff_rx.as_ref()
                .map(|diff_rx| sel.recv(diff_rx));
            let event_rx = sel.recv(&self.event_rx);
            let op = sel.select();
            match op.index() {
                i if Some(i) == diff_rx_index => match op.recv(self.diff_rx.as_ref().unwrap()) {
                    Ok(diff) => self.layers.ctx().diffs.append(diff),
                    Err(_) => {
                        self.layers.ctx().all_diffs_loaded = true;
                        self.diff_rx.take();
                    }
                }
                i if i == event_rx => match op.recv(&self.event_rx).unwrap() {
                    Event::Key(key_event) if key_event.kind == KeyEventKind::Press => {
                        self.layers.handle_key_event(key_event)
                    }
                    _ => {}
                }
                _ => unreachable!(),
            }
        }
    }
}

impl AppCtx {
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
        self.current_diff_index = match self.current_diff_index {
            None if self.diffs.is_empty() => None,
            None | Some(0) => Some(self.diffs.len().saturating_sub(1)),
            Some(index) => Some(index - 1),
        };
        self.center_diff();
    }
    fn next_diff(&mut self) {
        self.current_diff_index = match self.current_diff_index {
            None if self.diffs.is_empty() => None,
            Some(index) => Some((index + 1) % self.diffs.len()),
            None => Some(0),
        };
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

fn bench(args: Args) {
    let a = File::open(args.file1).unwrap();
    let b = File::open(args.file2).unwrap();
    match args.bench.unwrap() {
        Bench::Bytes => bench_iter(BytesDiffIter::new(a, b)),
        Bench::Memchr => bench_iter(MemchrDiffIter::new(a, b)),
        Bench::Threaded => bench_iter(ThreadedDiffIter::new(a, b)),
    }
}

fn bench_iter(iter: impl Iterator<Item = Range<u64>>) {
    let start = Instant::now();
    let mut count = 0;
    for diff in iter {
        println!("{diff:x?}");
        count += 1;
    }
    let elapsed = start.elapsed();
    println!("Found {count} diffs");
    eprintln!("Took {}:{}.{:03}", elapsed.as_secs() / 60, elapsed.as_secs() % 60, elapsed.subsec_millis());
}
