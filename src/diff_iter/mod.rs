mod bytes;
mod memchr;
mod threaded;

pub use bytes::BytesDiffIter;
pub use memchr::MemchrDiffIter;
pub use threaded::ThreadedDiffIter;

// bench on a 60GB file with 55 diffs (real broken RAID1 array)
// * bytes:    7min,   100% CPU =>  286 MB/s
// * memchr:   1min30s, 65% CPU => 1333 MB/s
// * threaded: 1min,   180% CPU => 2000 MB/s
