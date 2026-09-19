//! TitanFS: An Ultra-Fast, Crash-Consistent, Extent-Indexed Storage Engine.

pub mod types;
pub mod crc;
pub mod hash;
pub mod block_dev;
pub mod superblock;
pub mod bitmap;
pub mod extent;
pub mod inode;
pub mod directory;
pub mod journal;
pub mod cache;
pub mod dedup;
pub mod snapshot;
pub mod fs;
pub mod shell;
pub mod benchmark;

pub use types::*;
pub use block_dev::{BlockDevice, FileDevice, MemoryDevice};
pub use fs::{FileSystem, FormatOptions, FsckReport};
pub use shell::VfsShell;
pub use benchmark::{BenchmarkResults, BenchmarkSuite};
