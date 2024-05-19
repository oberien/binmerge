use std::fs::File;
use std::io::{BufReader, Bytes, Read};
use std::iter::Zip;
use std::ops::Range;

pub struct DiffIter {
    iter: Zip<Bytes<BufReader<File>>, Bytes<BufReader<File>>>,
    state: State,
}

#[derive(Debug, Copy, Clone)]
enum State {
    /// start, length
    Equal(u64, u64),
    /// start, length
    Different(u64, u64),
}

impl DiffIter {
    pub fn new(a: File, b: File) -> DiffIter {
        let a = BufReader::with_capacity(8*1024*1024, a);
        let b = BufReader::with_capacity(8*1024*1024, b);
        DiffIter {
            iter: a.bytes().zip(b.bytes()),
            state: State::Equal(0, 0),
        }
    }
}

impl Iterator for DiffIter {
    type Item = Range<u64>;

    fn next(&mut self) -> Option<Self::Item> {
        while let Some((a, b)) = self.iter.next() {
            let a = a.unwrap();
            let b = b.unwrap();
            self.state = match (a == b, self.state) {
                (true, State::Equal(start, len)) => State::Equal(start, len + 1),
                (true, State::Different(start, len_diff)) => {
                    self.state = State::Equal(start + len_diff, 1);
                    return Some(start..start + len_diff)
                }
                (false, State::Equal(start, len)) => State::Different(start + len, 1),
                (false, State::Different(start, len_diff)) => {
                    State::Different(start, len_diff + 1)
                },
            }
        }

        match self.state {
            State::Equal(..) => None,
            State::Different(start, len_diff) => {
                self.state = State::Equal(0, 0);
                Some(start..start + len_diff)
            }
        }
    }
}
