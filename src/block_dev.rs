//! Block device abstractions for TitanFS.
//! Provides memory-backed and disk file-backed block devices with extent read/write.

use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;
use crate::types::{BLOCK_SIZE, BlockId, FsError, Result};

/// Abstract interface for a block storage device.
pub trait BlockDevice: Send + Sync {
    /// Return the block size (typically 4096 bytes).
    fn block_size(&self) -> usize {
        BLOCK_SIZE
    }

    /// Total number of blocks addressable on this device.
    fn total_blocks(&self) -> u64;

    /// Read a single block into the provided buffer.
    fn read_block(&self, block_id: BlockId, buf: &mut [u8]) -> Result<()>;

    /// Write a single block from the provided buffer.
    fn write_block(&mut self, block_id: BlockId, buf: &[u8]) -> Result<()>;

    /// Read multiple contiguous blocks (extent I/O) in one operation.
    fn read_blocks(&self, start_block: BlockId, count: usize, buf: &mut [u8]) -> Result<()> {
        let bs = self.block_size();
        if buf.len() < count * bs {
            return Err(FsError::Other("Buffer too small for extent read".into()));
        }
        for i in 0..count {
            let b_id = start_block + i as u64;
            let offset = i * bs;
            self.read_block(b_id, &mut buf[offset..offset + bs])?;
        }
        Ok(())
    }

    /// Write multiple contiguous blocks (extent I/O) in one operation.
    fn write_blocks(&mut self, start_block: BlockId, count: usize, buf: &[u8]) -> Result<()> {
        let bs = self.block_size();
        if buf.len() < count * bs {
            return Err(FsError::Other("Buffer too small for extent write".into()));
        }
        for i in 0..count {
            let b_id = start_block + i as u64;
            let offset = i * bs;
            self.write_block(b_id, &buf[offset..offset + bs])?;
        }
        Ok(())
    }

    /// Flush any pending buffered writes to physical storage.
    fn flush(&mut self) -> Result<()>;
}

/// In-memory block device.
/// Stores blocks in a contiguous heap buffer for nanosecond access latency.
pub struct MemoryDevice {
    data: Vec<u8>,
    total_blocks: u64,
}

impl MemoryDevice {
    /// Create a new in-memory block device with the specified block count.
    pub fn new(total_blocks: u64) -> Self {
        let total_bytes = (total_blocks as usize) * BLOCK_SIZE;
        Self {
            data: vec![0u8; total_bytes],
            total_blocks,
        }
    }
}

impl BlockDevice for MemoryDevice {
    fn total_blocks(&self) -> u64 {
        self.total_blocks
    }

    fn read_block(&self, block_id: BlockId, buf: &mut [u8]) -> Result<()> {
        if block_id >= self.total_blocks {
            return Err(FsError::BlockOutOfBounds {
                block_id,
                total: self.total_blocks,
            });
        }
        let start = (block_id as usize) * BLOCK_SIZE;
        let end = start + BLOCK_SIZE;
        buf[..BLOCK_SIZE].copy_from_slice(&self.data[start..end]);
        Ok(())
    }

    fn write_block(&mut self, block_id: BlockId, buf: &[u8]) -> Result<()> {
        if block_id >= self.total_blocks {
            return Err(FsError::BlockOutOfBounds {
                block_id,
                total: self.total_blocks,
            });
        }
        let start = (block_id as usize) * BLOCK_SIZE;
        let end = start + BLOCK_SIZE;
        self.data[start..end].copy_from_slice(&buf[..BLOCK_SIZE]);
        Ok(())
    }

    fn read_blocks(&self, start_block: BlockId, count: usize, buf: &mut [u8]) -> Result<()> {
        let end_block = start_block + count as u64;
        if end_block > self.total_blocks {
            return Err(FsError::BlockOutOfBounds {
                block_id: end_block - 1,
                total: self.total_blocks,
            });
        }
        let start = (start_block as usize) * BLOCK_SIZE;
        let len = count * BLOCK_SIZE;
        buf[..len].copy_from_slice(&self.data[start..start + len]);
        Ok(())
    }

    fn write_blocks(&mut self, start_block: BlockId, count: usize, buf: &[u8]) -> Result<()> {
        let end_block = start_block + count as u64;
        if end_block > self.total_blocks {
            return Err(FsError::BlockOutOfBounds {
                block_id: end_block - 1,
                total: self.total_blocks,
            });
        }
        let start = (start_block as usize) * BLOCK_SIZE;
        let len = count * BLOCK_SIZE;
        self.data[start..start + len].copy_from_slice(&buf[..len]);
        Ok(())
    }

    fn flush(&mut self) -> Result<()> {
        Ok(())
    }
}

/// Disk file-backed block device for virtual disk images.
pub struct FileDevice {
    file: File,
    total_blocks: u64,
}

impl FileDevice {
    /// Create a new virtual disk image file with the specified block count.
    pub fn create<P: AsRef<Path>>(path: P, total_blocks: u64) -> Result<Self> {
        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(true)
            .open(path)?;

        let total_bytes = total_blocks * (BLOCK_SIZE as u64);
        file.set_len(total_bytes)?;
        file.flush()?;

        Ok(Self { file, total_blocks })
    }

    /// Open an existing virtual disk image file.
    pub fn open<P: AsRef<Path>>(path: P) -> Result<Self> {
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(path)?;

        let metadata = file.metadata()?;
        let total_bytes = metadata.len();
        let total_blocks = total_bytes / (BLOCK_SIZE as u64);

        if total_blocks == 0 {
            return Err(FsError::InvalidSuperblock("Disk image file is empty".into()));
        }

        Ok(Self { file, total_blocks })
    }
}

impl BlockDevice for FileDevice {
    fn total_blocks(&self) -> u64 {
        self.total_blocks
    }

    fn read_block(&self, block_id: BlockId, buf: &mut [u8]) -> Result<()> {
        if block_id >= self.total_blocks {
            return Err(FsError::BlockOutOfBounds {
                block_id,
                total: self.total_blocks,
            });
        }
        let offset = block_id * (BLOCK_SIZE as u64);
        let mut f = &self.file;
        f.seek(SeekFrom::Start(offset))?;
        f.read_exact(&mut buf[..BLOCK_SIZE])?;
        Ok(())
    }

    fn write_block(&mut self, block_id: BlockId, buf: &[u8]) -> Result<()> {
        if block_id >= self.total_blocks {
            return Err(FsError::BlockOutOfBounds {
                block_id,
                total: self.total_blocks,
            });
        }
        let offset = block_id * (BLOCK_SIZE as u64);
        self.file.seek(SeekFrom::Start(offset))?;
        self.file.write_all(&buf[..BLOCK_SIZE])?;
        Ok(())
    }

    fn read_blocks(&self, start_block: BlockId, count: usize, buf: &mut [u8]) -> Result<()> {
        let end_block = start_block + count as u64;
        if end_block > self.total_blocks {
            return Err(FsError::BlockOutOfBounds {
                block_id: end_block - 1,
                total: self.total_blocks,
            });
        }
        let offset = start_block * (BLOCK_SIZE as u64);
        let len = count * BLOCK_SIZE;
        let mut f = &self.file;
        f.seek(SeekFrom::Start(offset))?;
        f.read_exact(&mut buf[..len])?;
        Ok(())
    }

    fn write_blocks(&mut self, start_block: BlockId, count: usize, buf: &[u8]) -> Result<()> {
        let end_block = start_block + count as u64;
        if end_block > self.total_blocks {
            return Err(FsError::BlockOutOfBounds {
                block_id: end_block - 1,
                total: self.total_blocks,
            });
        }
        let offset = start_block * (BLOCK_SIZE as u64);
        let len = count * BLOCK_SIZE;
        self.file.seek(SeekFrom::Start(offset))?;
        self.file.write_all(&buf[..len])?;
        Ok(())
    }

    fn flush(&mut self) -> Result<()> {
        self.file.flush()?;
        Ok(())
    }
}
