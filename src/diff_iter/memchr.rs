use std::fs::File;
use std::io::{BufRead, BufReader};
use std::ops::Range;

pub struct MemchrDiffIter {
    a: BufReader<File>,
    b: BufReader<File>,
    pos: u64,
}

impl MemchrDiffIter {
    pub fn new(a: File, b: File) -> MemchrDiffIter {
        let a = BufReader::with_capacity(8*1024*1024, a);
        let b = BufReader::with_capacity(8*1024*1024, b);
        MemchrDiffIter { a, b, pos: 0 }
    }
}

impl Iterator for MemchrDiffIter {
    type Item = Range<u64>;

    fn next(&mut self) -> Option<Self::Item> {
        // get rid of equal bytes
        'outer: loop {
            let a = self.a.fill_buf().unwrap();
            let b = self.b.fill_buf().unwrap();
            let len = a.len().min(b.len());
            if len == 0 {
                return None;
            }

            let pos = a.iter().copied()
                .zip(b.iter().copied())
                .position(|(a, b)| a != b);
            match pos {
                Some(pos) => {
                    self.a.consume(pos);
                    self.b.consume(pos);
                    self.pos += pos as u64;
                    break 'outer;
                }
                None => {
                    self.a.consume(len);
                    self.b.consume(len);
                    self.pos += len as u64;
                }
            }
        }

        // we found a diff
        let start = self.pos;
        loop {
            let a = self.a.fill_buf().unwrap();
            let b = self.b.fill_buf().unwrap();
            let len = a.len().min(b.len());
            if len == 0 {
                return Some(start..self.pos);
            }

            let pos = a.iter().copied()
                .zip(b.iter().copied())
                .position(|(a, b)| a == b);
            match pos {
                Some(pos) => {
                    self.a.consume(pos);
                    self.b.consume(pos);
                    self.pos += pos as u64;
                    return Some(start..self.pos);
                }
                None => {
                    self.a.consume(len);
                    self.b.consume(len);
                    self.pos += len as u64;
                }
            }
        }
    }
}
