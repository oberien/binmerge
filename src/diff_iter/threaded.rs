use std::collections::VecDeque;
use std::fs::File;
use std::io::Read;
use std::ops::Range;
use std::thread;
use crossbeam_channel::{Receiver, Sender};

pub struct ThreadedDiffIter {
    arx: Receiver<Vec<u8>>,
    brx: Receiver<Vec<u8>>,
    a: VecDeque<u8>,
    b: VecDeque<u8>,
    pos: u64,
}

impl ThreadedDiffIter {
    pub fn new(a: File, b: File) -> ThreadedDiffIter {
        let (atx, arx) = crossbeam_channel::bounded(64);
        let (btx, brx) = crossbeam_channel::bounded(64);
        fn thread_fn(file: File, tx: Sender<Vec<u8>>) {
            let file = &file;
            loop {
                let mut buf = Vec::with_capacity(8*1024*1024);
                let read = file.take(8*1024*1024).read_to_end(&mut buf).unwrap();
                if read == 0 { break; }
                tx.send(buf).unwrap();
            }
        }
        thread::spawn(move || thread_fn(a, atx));
        thread::spawn(move || thread_fn(b, btx));
        ThreadedDiffIter {
            arx,
            brx,
            a: VecDeque::new(),
            b: VecDeque::new(),
            pos: 0,
        }
    }

    fn fill_buffs(&mut self) -> Option<(&mut VecDeque<u8>, &mut VecDeque<u8>)>{
        if self.a.is_empty() {
            self.a = VecDeque::from(self.arx.recv().ok()?);
        }
        if self.b.is_empty() {
            self.b = VecDeque::from(self.brx.recv().ok()?);
        }
        Some((&mut self.a, &mut self.b))
    }
    fn consume(&mut self, amount: usize) {
        drop(self.a.drain(..amount));
        drop(self.b.drain(..amount));
        self.pos += amount as u64;
    }
}

impl Iterator for ThreadedDiffIter {
    type Item = Range<u64>;

    fn next(&mut self) -> Option<Self::Item> {
        'outer: loop {

            // get rid of equal bytes
            'equal: loop {
                let (a, b) = self.fill_buffs()?;
                let len = a.len();
                let pos = a.iter().copied()
                    .zip(b.iter().copied())
                    .position(|(a, b)| a != b);
                match pos {
                    Some(pos) => {
                        self.consume(pos);
                        break 'equal;
                    }
                    None => {
                        self.consume(len);
                        continue 'outer;
                    }
                }
            }

            // we found a diff
            let start = self.pos;
            loop {
                let (a, b) = match self.fill_buffs() {
                    Some((a, b)) => (a, b),
                    None => return Some(start..self.pos),
                };
                let len = a.len();

                let pos = a.iter().copied()
                    .zip(b.iter().copied())
                    .position(|(a, b)| a == b);
                match pos {
                    Some(pos) => {
                        self.consume(pos);
                        return Some(start..self.pos);
                    }
                    None => {
                        self.consume(len);
                    }
                }
            }
        }
    }
}
