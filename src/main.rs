//! TitanFS CLI Command Suite.

use std::env;
use std::process;
use titanfs::block_dev::FileDevice;
use titanfs::fs::{FileSystem, FormatOptions};
use titanfs::shell::VfsShell;
use titanfs::benchmark::BenchmarkSuite;

fn print_usage() {
    println!("TitanFS - Next-Generation Ultra-Fast Filesystem");
    println!("Usage:");
    println!("  titanfs mkfs <image_path> [--size-mb <N>]   Format a new virtual disk image");
    println!("  titanfs shell <image_path>                  Mount image and start interactive shell");
    println!("  titanfs fsck <image_path>                   Check filesystem consistency and integrity");
    println!("  titanfs info <image_path>                   Display superblock and geometry info");
    println!("  titanfs bench [--size-mb <N>]               Run full performance benchmark suite");
    println!("  titanfs help                                Show this help menu");
}

fn main() {
    let args: Vec<String> = env::args().collect();
    if args.len() < 2 {
        print_usage();
        process::exit(1);
    }

    let command = args[1].as_str();
    match command {
        "mkfs" => {
            if args.len() < 3 {
                println!("Error: missing image path.");
                println!("Usage: titanfs mkfs <image_path> [--size-mb <N>]");
                process::exit(1);
            }
            let path = &args[2];
            let mut size_mb = 128u64; // default 128 MB

            let mut idx = 3;
            while idx < args.len() {
                if args[idx] == "--size-mb" && idx + 1 < args.len() {
                    size_mb = args[idx + 1].parse().unwrap_or(128);
                    idx += 1;
                }
                idx += 1;
            }

            let total_blocks = (size_mb * 1024 * 1024) / 4096;
            println!("Creating virtual disk image '{}' ({} MB, {} blocks)...", path, size_mb, total_blocks);

            let dev = match FileDevice::create(path, total_blocks) {
                Ok(d) => d,
                Err(e) => {
                    eprintln!("Failed to create disk image: {}", e);
                    process::exit(1);
                }
            };

            match FileSystem::format(
                dev,
                FormatOptions {
                    total_blocks,
                    total_inodes: total_blocks / 4,
                    enable_dedup: true,
                },
            ) {
                Ok(mut fs) => {
                    let _ = fs.sync();
                    println!("TitanFS formatted successfully on '{}'!", path);
                }
                Err(e) => {
                    eprintln!("Format error: {}", e);
                    process::exit(1);
                }
            }
        }
        "shell" => {
            if args.len() < 3 {
                println!("Error: missing image path.");
                println!("Usage: titanfs shell <image_path>");
                process::exit(1);
            }
            let path = &args[2];
            println!("Opening disk image '{}'...", path);
            let dev = match FileDevice::open(path) {
                Ok(d) => d,
                Err(e) => {
                    eprintln!("Failed to open image '{}': {}", path, e);
                    process::exit(1);
                }
            };

            let fs = match FileSystem::mount(dev) {
                Ok(f) => f,
                Err(e) => {
                    eprintln!("Mount error: {}", e);
                    process::exit(1);
                }
            };

            let mut shell = VfsShell::new(fs);
            if let Err(e) = shell.run() {
                eprintln!("Shell error: {}", e);
            }
        }
        "fsck" => {
            if args.len() < 3 {
                println!("Error: missing image path.");
                println!("Usage: titanfs fsck <image_path>");
                process::exit(1);
            }
            let path = &args[2];
            let dev = match FileDevice::open(path) {
                Ok(d) => d,
                Err(e) => {
                    eprintln!("Failed to open image '{}': {}", path, e);
                    process::exit(1);
                }
            };

            let mut fs = match FileSystem::mount(dev) {
                Ok(f) => f,
                Err(e) => {
                    eprintln!("Mount error: {}", e);
                    process::exit(1);
                }
            };

            match fs.fsck() {
                Ok(report) => {
                    println!("============================================================");
                    println!("             TitanFS Filesystem Consistency Report");
                    println!("============================================================");
                    println!("  Target Image:     {}", path);
                    println!("  Status:           {}", if report.is_clean { "HEALTHY / CLEAN" } else { "CORRUPTED" });
                    println!("  Total Blocks:     {}", report.total_blocks);
                    println!("  Used Blocks:      {}", report.used_blocks);
                    println!("  Free Blocks:      {}", report.free_blocks);
                    println!("  Total Inodes:     {}", report.total_inodes);
                    println!("  Used Inodes:      {}", report.used_inodes);
                    println!("  Corrupted Blocks: {}", report.corrupted_blocks);
                    println!("  Orphan Inodes:    {}", report.orphan_inodes);
                    println!("============================================================");
                }
                Err(e) => eprintln!("fsck failed: {}", e),
            }
        }
        "info" => {
            if args.len() < 3 {
                println!("Error: missing image path.");
                println!("Usage: titanfs info <image_path>");
                process::exit(1);
            }
            let path = &args[2];
            let dev = match FileDevice::open(path) {
                Ok(d) => d,
                Err(e) => {
                    eprintln!("Failed to open image '{}': {}", path, e);
                    process::exit(1);
                }
            };

            let fs = match FileSystem::mount(dev) {
                Ok(f) => f,
                Err(e) => {
                    eprintln!("Mount error: {}", e);
                    process::exit(1);
                }
            };

            let sb = &fs.sb;
            println!("============================================================");
            println!("                TitanFS Superblock Geometry");
            println!("============================================================");
            println!("  Magic:             0x{:016X}", sb.magic);
            println!("  Version:           {}", sb.version);
            println!("  Block Size:        {} bytes", sb.block_size);
            println!("  Total Blocks:      {} ({} MB)", sb.total_blocks, (sb.total_blocks * 4096) / (1024 * 1024));
            println!("  Free Blocks:       {}", sb.free_blocks);
            println!("  Total Inodes:      {}", sb.total_inodes);
            println!("  Free Inodes:       {}", sb.free_inodes);
            println!("  Root Inode:        {}", sb.root_inode);
            println!("  Journal Region:    Blocks {}..{}", sb.journal_start_block, sb.journal_start_block + sb.journal_block_count);
            println!("  Block Bitmap:      Blocks {}..{}", sb.block_bitmap_start, sb.block_bitmap_start + sb.block_bitmap_blocks);
            println!("  Inode Bitmap:      Blocks {}..{}", sb.inode_bitmap_start, sb.inode_bitmap_start + sb.inode_bitmap_blocks);
            println!("  Inode Table:       Blocks {}..{}", sb.inode_table_start, sb.inode_table_start + sb.inode_table_blocks);
            println!("  Data Region:       Blocks {}..{}", sb.data_start_block, sb.data_start_block + sb.data_block_count);
            println!("  Generation:        {}", sb.generation);
            println!("  Mount Count:       {}", sb.mount_count);
            println!("  Superblock CRC:    0x{:08X}", sb.crc);
            println!("============================================================");
        }
        "bench" => {
            let mut size_mb = 64usize;
            if args.len() >= 4 && args[2] == "--size-mb" {
                size_mb = args[3].parse().unwrap_or(64);
            }
            BenchmarkSuite::run_benchmarks(size_mb);
        }
        "help" | "--help" | "-h" => {
            print_usage();
        }
        _ => {
            println!("Unknown command: '{}'", command);
            print_usage();
            process::exit(1);
        }
    }
}
