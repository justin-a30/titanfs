//! Core types, constants, and error definitions for TitanFS.

use std::fmt;

/// Standard filesystem block size in bytes (4 KB).
pub const BLOCK_SIZE: usize = 4096;

/// Magic signature for TitanFS superblocks: "TITANFS1"
pub const TITANFS_MAGIC: u64 = 0x544954414E465331;

/// Current filesystem format version.
pub const TITANFS_VERSION: u32 = 1;

/// Root directory inode ID.
pub const ROOT_INODE_ID: u64 = 1;

/// Size of an on-disk inode record in bytes.
pub const INODE_SIZE: usize = 256;

/// Number of inodes per 4096-byte block.
pub const INODES_PER_BLOCK: usize = BLOCK_SIZE / INODE_SIZE; // 16

/// Maximum inline data size stored directly inside the inode record (in bytes).
pub const INLINE_DATA_MAX: usize = 128;

/// Maximum length for file and directory names (in bytes).
pub const MAX_NAME_LEN: usize = 255;

/// Inode flags.
pub const FLAG_INLINE_DATA: u32 = 0x0001;
pub const FLAG_COW_SHARED:  u32 = 0x0002;
pub const FLAG_DIRECTORY:   u32 = 0x0004;

/// Block identifier.
pub type BlockId = u64;

/// Inode identifier.
pub type InodeId = u64;

/// File type enumeration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum FileType {
    Unknown = 0,
    RegularFile = 1,
    Directory = 2,
    Symlink = 3,
}

impl FileType {
    pub fn from_u8(val: u8) -> Self {
        match val {
            1 => FileType::RegularFile,
            2 => FileType::Directory,
            3 => FileType::Symlink,
            _ => FileType::Unknown,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            FileType::RegularFile => "FILE",
            FileType::Directory => "DIR",
            FileType::Symlink => "LINK",
            FileType::Unknown => "UNK",
        }
    }
}

/// TitanFS error variants.
#[derive(Debug, Clone)]
pub enum FsError {
    BlockOutOfBounds { block_id: BlockId, total: u64 },
    DiskFull,
    InodeTableFull,
    FileNotFound(String),
    AlreadyExists(String),
    NotADirectory(String),
    IsADirectory(String),
    DirectoryNotEmpty(String),
    InvalidPath(String),
    CorruptedBlock { block_id: BlockId, expected: u32, actual: u32 },
    InvalidSuperblock(String),
    InvalidInode(InodeId),
    JournalCorrupted(String),
    SnapshotNotFound(String),
    Io(String),
    Other(String),
}

impl fmt::Display for FsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FsError::BlockOutOfBounds { block_id, total } => {
                write!(f, "Block index out of bounds: {} (total: {})", block_id, total)
            }
            FsError::DiskFull => write!(f, "No free blocks remaining on device (Disk Full)"),
            FsError::InodeTableFull => write!(f, "No free inodes remaining (Inode Table Full)"),
            FsError::FileNotFound(path) => write!(f, "No such file or directory: '{}'", path),
            FsError::AlreadyExists(path) => write!(f, "File or directory already exists: '{}'", path),
            FsError::NotADirectory(path) => write!(f, "Not a directory: '{}'", path),
            FsError::IsADirectory(path) => write!(f, "Is a directory: '{}'", path),
            FsError::DirectoryNotEmpty(path) => write!(f, "Directory is not empty: '{}'", path),
            FsError::InvalidPath(path) => write!(f, "Invalid path: '{}'", path),
            FsError::CorruptedBlock { block_id, expected, actual } => {
                write!(f, "Block {} CRC32 mismatch (expected 0x{:08X}, got 0x{:08X})", block_id, expected, actual)
            }
            FsError::InvalidSuperblock(msg) => write!(f, "Invalid superblock: {}", msg),
            FsError::InvalidInode(id) => write!(f, "Invalid inode: {}", id),
            FsError::JournalCorrupted(msg) => write!(f, "Journal corruption: {}", msg),
            FsError::SnapshotNotFound(name) => write!(f, "Snapshot not found: '{}'", name),
            FsError::Io(err) => write!(f, "Underlying I/O error: {}", err),
            FsError::Other(msg) => write!(f, "Filesystem error: {}", msg),
        }
    }
}

impl std::error::Error for FsError {}

impl From<std::io::Error> for FsError {
    fn from(err: std::io::Error) -> Self {
        FsError::Io(err.to_string())
    }
}

pub type Result<T> = std::result::Result<T, FsError>;
