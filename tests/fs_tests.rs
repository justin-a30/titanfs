//! Comprehensive Integration & Unit Test Suite for TitanFS.

use std::fs;
use titanfs::block_dev::{FileDevice, MemoryDevice};
use titanfs::fs::{FileSystem, FormatOptions};
use titanfs::superblock::Superblock;
use titanfs::types::{ROOT_INODE_ID, TITANFS_MAGIC, TITANFS_VERSION};

#[test]
fn test_superblock_roundtrip() {
    let sb = Superblock::new(65536, 16384);
    assert_eq!(sb.magic, TITANFS_MAGIC);
    assert_eq!(sb.version, TITANFS_VERSION);

    let mut buf = [0u8; 4096];
    sb.serialize(&mut buf);

    let deserialized = Superblock::deserialize(&buf).expect("Failed to deserialize superblock");
    assert_eq!(sb.magic, deserialized.magic);
    assert_eq!(sb.total_blocks, deserialized.total_blocks);
    assert_eq!(sb.root_inode, ROOT_INODE_ID);
    assert_eq!(sb.crc, deserialized.crc);
}

#[test]
fn test_small_file_inlining() {
    let dev = MemoryDevice::new(1024);
    let mut fs = FileSystem::format(dev, FormatOptions::default()).unwrap();

    let path = "/inline_test.txt";
    let content = b"Hello TitanFS! Ultra-fast inlined inode data.";
    fs.create_file(path, 0o644).unwrap();
    fs.write_file(path, 0, content).unwrap();

    let inode = fs.stat(path).unwrap();
    assert!(inode.is_inline(), "Small file must be stored inline in inode!");
    assert_eq!(inode.size, content.len() as u64);

    let read_back = fs.read_file(path, 0, 100).unwrap();
    assert_eq!(read_back, content);
}

#[test]
fn test_large_file_extent_allocation() {
    let dev = MemoryDevice::new(16384); // 64 MB
    let mut fs = FileSystem::format(dev, FormatOptions::default()).unwrap();

    let path = "/large_file.dat";
    let size = 2 * 1024 * 1024; // 2 MB
    let mut data = Vec::with_capacity(size);
    for i in 0..size {
        data.push((i % 251) as u8);
    }

    fs.create_file(path, 0o644).unwrap();
    fs.write_file(path, 0, &data).unwrap();

    let inode = fs.stat(path).unwrap();
    assert!(!inode.is_inline(), "2MB file must not be inline");
    assert_eq!(inode.size, size as u64);
    assert!(inode.allocated_blocks >= 512);

    let read_back = fs.read_file(path, 0, size).unwrap();
    assert_eq!(read_back, data, "Read data must match written data exactly");
}

#[test]
fn test_nested_directories_and_traversal() {
    let dev = MemoryDevice::new(4096);
    let mut fs = FileSystem::format(dev, FormatOptions::default()).unwrap();

    fs.create_dir("/deep").unwrap();
    fs.create_dir("/deep/nested").unwrap();
    fs.create_dir("/deep/nested/path").unwrap();

    let file_path = "/deep/nested/path/sample.log";
    fs.create_file(file_path, 0o644).unwrap();
    fs.write_file(file_path, 0, b"log line 1\nlog line 2").unwrap();

    let inode = fs.stat(file_path).unwrap();
    assert_eq!(inode.size, 21);

    let entries = fs.list_dir("/deep/nested/path").unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].name, "sample.log");
}

#[test]
fn test_cow_snapshot_and_rollback() {
    let dev = MemoryDevice::new(4096);
    let mut fs = FileSystem::format(dev, FormatOptions::default()).unwrap();

    let path = "/state.txt";
    fs.create_file(path, 0o644).unwrap();
    fs.write_file(path, 0, b"Initial State v1").unwrap();

    // Create snapshot
    fs.create_snapshot("snap1").unwrap();

    // Modify file
    fs.write_file(path, 0, b"Modified State v2").unwrap();
    let current_data = fs.read_file(path, 0, 50).unwrap();
    assert_eq!(current_data, b"Modified State v2");

    // Rollback to snapshot
    fs.rollback_snapshot("snap1").unwrap();
    let rolled_back_data = fs.read_file(path, 0, 50).unwrap();
    assert_eq!(rolled_back_data, b"Initial State v1", "Rollback must restore original contents");
}

#[test]
fn test_deduplication_engine() {
    let dev = MemoryDevice::new(8192);
    let mut fs = FileSystem::format(
        dev,
        FormatOptions {
            total_blocks: 8192,
            total_inodes: 2048,
            enable_dedup: true,
        },
    ).unwrap();

    let block_data = vec![0x42u8; 4096];
    fs.create_file("/f1.bin", 0o644).unwrap();
    fs.write_file("/f1.bin", 0, &block_data).unwrap();

    fs.create_file("/f2.bin", 0o644).unwrap();
    fs.write_file("/f2.bin", 0, &block_data).unwrap();

    assert!(fs.dedup.saved_blocks() >= 1, "Identical blocks must trigger deduplication");

    let r1 = fs.read_file("/f1.bin", 0, 4096).unwrap();
    let r2 = fs.read_file("/f2.bin", 0, 4096).unwrap();
    assert_eq!(r1, block_data);
    assert_eq!(r2, block_data);
}

#[test]
fn test_file_device_persistence_and_fsck() {
    let temp_img = std::env::temp_dir().join("titanfs_test_disk.tfs");
    if temp_img.exists() {
        let _ = fs::remove_file(&temp_img);
    }

    // 1. Create and format image
    {
        let dev = FileDevice::create(&temp_img, 4096).unwrap();
        let mut fs = FileSystem::format(dev, FormatOptions::default()).unwrap();
        fs.create_file("/persisted.txt", 0o644).unwrap();
        fs.write_file("/persisted.txt", 0, b"Persistent storage test on disk image!").unwrap();
        fs.sync().unwrap();
    }

    // 2. Re-mount and verify
    {
        let dev = FileDevice::open(&temp_img).unwrap();
        let mut fs = FileSystem::mount(dev).unwrap();

        let data = fs.read_file("/persisted.txt", 0, 100).unwrap();
        assert_eq!(data, b"Persistent storage test on disk image!");

        let report = fs.fsck().unwrap();
        assert!(report.is_clean, "Fsck must report clean filesystem");
        assert_eq!(report.corrupted_blocks, 0);
    }

    // Clean up test file
    let _ = fs::remove_file(&temp_img);
}
