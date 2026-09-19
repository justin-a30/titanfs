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

#[test]
fn test_chronovfs_microsecond_time_travel() {
    let dev = MemoryDevice::new(4096);
    let mut fs = FileSystem::format(dev, FormatOptions::default()).unwrap();

    let path = "/temporal_doc.txt";
    fs.create_file(path, 0o644).unwrap();

    // Rev 1
    fs.write_file_truncate(path, b"Revision 1: Initial draft.").unwrap();
    std::thread::sleep(std::time::Duration::from_millis(60));

    // Rev 2
    fs.write_file_truncate(path, b"Revision 2: Updated draft with new notes.").unwrap();
    std::thread::sleep(std::time::Duration::from_millis(60));

    // Rev 3
    fs.write_file_truncate(path, b"Revision 3: Final published edition.").unwrap();

    // Query current head
    let head = fs.read_file(path, 0, 100).unwrap();
    assert_eq!(head, b"Revision 3: Final published edition.");

    // Query -50ms (should resolve to Rev 2)
    let rev2 = fs.read_file("/temporal_doc.txt@-50ms", 0, 100).unwrap();
    assert_eq!(rev2, b"Revision 2: Updated draft with new notes.");

    // Query -110ms (should resolve to Rev 1)
    let rev1 = fs.read_file("/temporal_doc.txt@-110ms", 0, 100).unwrap();
    assert_eq!(rev1, b"Revision 1: Initial draft.");

    // Check version timeline listing
    let inode = fs.stat(path).unwrap();
    let versions = fs.chrono.list_versions(inode.id);
    assert!(versions.len() >= 3, "ChronoVFS must track all revisions");
}

#[test]
fn test_ghost_trees_instant_workspace_branching() {
    let dev = MemoryDevice::new(8192);
    let mut fs = FileSystem::format(dev, FormatOptions::default()).unwrap();

    // Set up source directory workspace
    fs.create_dir("/project").unwrap();
    fs.create_file("/project/main.rs", 0o644).unwrap();
    fs.write_file("/project/main.rs", 0, b"fn main() { println!(\"Production\"); }").unwrap();
    fs.create_file("/project/config.toml", 0o644).unwrap();
    fs.write_file("/project/config.toml", 0, b"debug = false").unwrap();

    let blocks_before = fs.sb.total_blocks - fs.block_bitmap.count_free();

    // Instant O(1) workspace branch
    fs.branch_dir("/project", "/project-feature").unwrap();

    let blocks_after = fs.sb.total_blocks - fs.block_bitmap.count_free();
    // Only 1 new directory inode allocated, zero file data blocks duplicated!
    assert!(blocks_after <= blocks_before + 1, "GhostTrees must branch in O(1) zero block overhead");

    // Verify branch registration and Merkle root
    let branch_meta = fs.ghost_trees.get_branch("project-feature").expect("Branch must be registered");
    assert_ne!(branch_meta.merkle_root, 0, "Merkle root must be computed");

    // Verify files exist in branched workspace
    let branch_entries = fs.list_dir("/project-feature").unwrap();
    assert_eq!(branch_entries.len(), 2);

    let branch_data = fs.read_file("/project-feature/main.rs", 0, 100).unwrap();
    assert_eq!(branch_data, b"fn main() { println!(\"Production\"); }");

    // Modify branch - ensure trunk remains isolated
    fs.write_file("/project-feature/main.rs", 0, b"fn main() { println!(\"Feature Test\"); }").unwrap();

    let prod_data = fs.read_file("/project/main.rs", 0, 100).unwrap();
    let feat_data = fs.read_file("/project-feature/main.rs", 0, 100).unwrap();
    assert_eq!(prod_data, b"fn main() { println!(\"Production\"); }");
    assert_eq!(feat_data, b"fn main() { println!(\"Feature Test\"); }");
}

#[test]
fn test_ransom_shield_entropy_detection_and_quarantine() {
    use titanfs::ransom_shield::calculate_shannon_entropy;

    // Verify entropy calculation
    let plain_text = b"This is plain English text containing normal words and letters.";
    let plain_entropy = calculate_shannon_entropy(plain_text);
    assert!(plain_entropy < 5.0, "Plain text entropy must be < 5.0, got: {}", plain_entropy);

    // Generate high-entropy pseudo-random ciphertext (AES-style)
    let mut encrypted_payload = vec![0u8; 4096];
    let mut state = 0x12345678u64;
    for b in &mut encrypted_payload {
        state = state.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        *b = (state >> 32) as u8;
    }
    let encrypted_entropy = calculate_shannon_entropy(&encrypted_payload);
    assert!(encrypted_entropy > 7.9, "Pseudo-random encrypted payload entropy must be > 7.9, got: {}", encrypted_entropy);

    // Test autonomous detection in FileSystem
    let dev = MemoryDevice::new(8192);
    let mut fs = FileSystem::format(dev, FormatOptions::default()).unwrap();

    // Benign writes
    fs.create_file("/doc1.txt", 0o644).unwrap();
    fs.write_file("/doc1.txt", 0, plain_text).unwrap();
    assert!(!fs.ransom_shield.is_quarantine_active());

    // Attack simulation: Rapid high-entropy burst encryption across 3 files
    fs.create_file("/doc1.txt.enc", 0o644).unwrap();
    fs.write_file("/doc1.txt.enc", 0, &encrypted_payload).unwrap();

    fs.create_file("/doc2.txt.enc", 0o644).unwrap();
    fs.write_file("/doc2.txt.enc", 0, &encrypted_payload).unwrap();

    fs.create_file("/doc3.txt.enc", 0o644).unwrap();
    fs.write_file("/doc3.txt.enc", 0, &encrypted_payload).unwrap();

    // Quorum threshold (3 distinct files) reached -> Lockdown triggered!
    assert!(fs.ransom_shield.is_quarantine_active(), "RansomShield must trigger lockdown on burst attack!");

    let report = fs.ransom_shield.get_report();
    assert!(report.locked);
    assert!(report.quarantined_blocks >= 1);

    // Unlock and discard
    let purged = fs.ransom_shield.discard_quarantine();
    assert!(purged >= 1);
    assert!(!fs.ransom_shield.is_quarantine_active());
}

#[test]
fn test_omni_index_instant_compound_search() {
    use titanfs::omni_index::{SearchQuery, SizeBracket};

    let dev = MemoryDevice::new(8192);
    let mut fs = FileSystem::format(dev, FormatOptions::default()).unwrap();

    fs.create_dir("/src").unwrap();
    fs.create_file("/src/main.rs", 0o644).unwrap();
    fs.write_file("/src/main.rs", 0, &vec![0xAA; 1500]).unwrap(); // 1500 B

    fs.create_file("/src/lib.rs", 0o644).unwrap();
    fs.write_file("/src/lib.rs", 0, &vec![0xBB; 400]).unwrap(); // 400 B (Tiny)

    fs.create_dir("/docs").unwrap();
    fs.create_file("/docs/readme.md", 0o644).unwrap();
    fs.write_file("/docs/readme.md", 0, b"# TitanFS Documentation").unwrap();

    fs.create_file("/docs/spec.pdf", 0o644).unwrap();
    fs.write_file("/docs/spec.pdf", 0, &vec![0xCC; 80000]).unwrap(); // 80 KB (Medium)

    // 1. Search by extension
    let rs_query = SearchQuery {
        extension: Some("rs"),
        ..Default::default()
    };
    let rs_results = fs.search_index(&rs_query);
    assert_eq!(rs_results.len(), 2, "Must find exactly 2 .rs files");

    // 2. Search by size range
    let size_query = SearchQuery {
        min_size: Some(1000),
        max_size: Some(2000),
        ..Default::default()
    };
    let size_results = fs.search_index(&size_query);
    assert_eq!(size_results.len(), 1);
    assert_eq!(size_results[0].path, "/src/main.rs");

    // 3. Search by bracket
    assert_eq!(SizeBracket::from_size(400), SizeBracket::Tiny);
    assert_eq!(SizeBracket::from_size(1500), SizeBracket::Small);
    assert_eq!(SizeBracket::from_size(80000), SizeBracket::Medium);

    // 4. Unindexing on file deletion
    fs.remove_file("/src/lib.rs").unwrap();
    let rs_after = fs.search_index(&rs_query);
    assert_eq!(rs_after.len(), 1, "Deleted file must be removed from OmniIndex");
}
