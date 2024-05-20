use std::mem;
use std::ops::Range;
use positioned_io::{RandomAccessFile, ReadAt, WriteAt};
use crate::{AppCtx, restore_terminal};

pub fn apply_changes(ctx: &mut AppCtx) {
    restore_terminal();
    let merges_1_into_2 = mem::take(&mut ctx.merges_1_into_2);
    let merges_2_into_1 = mem::take(&mut ctx.merges_2_into_1);
    let len_1into2 = merges_1_into_2.len();
    let len_2into1 = merges_2_into_1.len();
    let mut done = 0;
    println!("Starting merge");
    for (i, range) in merges_2_into_1.into_inner().into_iter().enumerate() {
        copy(&ctx.file2, &mut ctx.file1, range);
        done += 1;
        println!("Merged left  {:>3} / {}, Total {:>3} / {}", i+1, len_2into1, done, len_1into2 + len_2into1);
    }
    for (i, range) in merges_1_into_2.into_inner().into_iter().enumerate() {
        copy(&ctx.file1, &mut ctx.file2, range);
        done += 1;
        println!("Merged right {:>3} / {}, Total {:>3} / {}", i+1, len_2into1, done, len_1into2 + len_2into1);
    }
    println!("Done");
    std::process::exit(0);
}

fn copy(from: &RandomAccessFile, to: &mut RandomAccessFile, range: Range<u64>) {
    let mut buf = vec![0u8; 8*1024*1024];
    let mut pos = range.start;

    while pos < range.end {
        let size = buf.len().min((range.end - pos) as usize);
        let read = from.read_at(pos, &mut buf[..size]).unwrap();
        to.write_all_at(pos, &buf[..read]).unwrap();
        pos += read as u64;
    }
}
