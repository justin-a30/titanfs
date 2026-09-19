//! Comprehensive multi-workload benchmark suite for TitanFS and comparison vs host OS.

use std::fs::{self, File};
use std::io::{Read, Write};
use std::time::Instant;
use crate::block_dev::MemoryDevice;
use crate::fs::{FileSystem, FormatOptions};

pub struct BenchmarkResults {
    pub seq_write_mb_s: f64,
    pub seq_read_mb_s: f64,
    pub rand_write_iops: f64,
    pub rand_read_iops: f64,
    pub small_file_create_per_sec: f64,
    pub dir_traversal_per_sec: f64,
    pub dedup_efficiency_pct: f64,

    // Host comparison metrics
    pub host_seq_write_mb_s: f64,
    pub host_seq_read_mb_s: f64,
    pub host_small_file_per_sec: f64,
}

pub struct BenchmarkSuite;

impl BenchmarkSuite {
    /// Run full suite of benchmarks on TitanFS and compare with host filesystem.
    pub fn run_benchmarks(size_mb: usize) -> BenchmarkResults {
        println!("============================================================");
        println!("            TitanFS Ultra-Performance Benchmark");
        println!("============================================================");
        println!("Test dataset size: {} MB", size_mb);

        let total_blocks = ((size_mb * 1024 * 1024) / 4096) as u64 * 4 + 32768;
        let mem_dev = MemoryDevice::new(total_blocks);
        let mut fs = FileSystem::format(
            mem_dev,
            FormatOptions {
                total_blocks,
                total_inodes: 65536,
                enable_dedup: true,
            },
        ).expect("Failed to format TitanFS in-memory instance");

        // 1. Sequential Write Benchmark
        print!("Running Sequential Write Benchmark ({} MB)... ", size_mb);
        let write_buf = vec![0xABu8; 1024 * 1024]; // 1 MB buffer
        let _ = fs.create_file("/seq_test.bin", 0o644);

        let start = Instant::now();
        for i in 0..size_mb {
            let offset = (i as u64) * (1024 * 1024);
            fs.write_file("/seq_test.bin", offset, &write_buf).expect("Write failed");
        }
        fs.sync().expect("Sync failed");
        let write_dur = start.elapsed();
        let seq_write_mb_s = (size_mb as f64) / write_dur.as_secs_f64();
        println!("{:.2} MB/s ({:.2} GB/s)", seq_write_mb_s, seq_write_mb_s / 1024.0);

        // 2. Sequential Read Benchmark
        print!("Running Sequential Read Benchmark ({} MB)... ", size_mb);
        let start = Instant::now();
        for i in 0..size_mb {
            let offset = (i as u64) * (1024 * 1024);
            let chunk = fs.read_file("/seq_test.bin", offset, 1024 * 1024).expect("Read failed");
            assert_eq!(chunk.len(), 1024 * 1024);
        }
        let read_dur = start.elapsed();
        let seq_read_mb_s = (size_mb as f64) / read_dur.as_secs_f64();
        println!("{:.2} MB/s ({:.2} GB/s)", seq_read_mb_s, seq_read_mb_s / 1024.0);

        // 3. Random 4KB Write IOPS Benchmark
        let random_ops = 50_000;
        print!("Running Random 4KB Write Benchmark ({} IOPS test)... ", random_ops);
        let block_4k = [0x55u8; 4096];
        let _ = fs.create_file("/rand_test.bin", 0o644);
        let rand_file_mb = size_mb.min(16);
        for i in 0..rand_file_mb {
            let offset = (i as u64) * (1024 * 1024);
            fs.write_file("/rand_test.bin", offset, &write_buf).expect("Pre-alloc failed");
        }
        let total_rand_blocks = (rand_file_mb * 1024 * 1024) / 4096;

        let start = Instant::now();
        for i in 0..random_ops {
            let offset = ((i * 7919) % total_rand_blocks) as u64 * 4096;
            fs.write_file("/rand_test.bin", offset, &block_4k).expect("Random write failed");
        }
        fs.sync().expect("Sync failed");
        let rand_w_dur = start.elapsed();
        let rand_write_iops = (random_ops as f64) / rand_w_dur.as_secs_f64();
        let rand_w_lat_us = (rand_w_dur.as_micros() as f64) / (random_ops as f64);
        println!("{:.0} IOPS (avg latency: {:.2} µs)", rand_write_iops, rand_w_lat_us);

        // 4. Random 4KB Read IOPS Benchmark
        print!("Running Random 4KB Read Benchmark ({} IOPS test)... ", random_ops);
        let start = Instant::now();
        for i in 0..random_ops {
            let offset = ((i * 7919) % total_rand_blocks) as u64 * 4096;
            let _ = fs.read_file("/rand_test.bin", offset, 4096).expect("Random read failed");
        }
        let rand_r_dur = start.elapsed();
        let rand_read_iops = (random_ops as f64) / rand_r_dur.as_secs_f64();
        let rand_r_lat_us = (rand_r_dur.as_micros() as f64) / (random_ops as f64);
        println!("{:.0} IOPS (avg latency: {:.2} µs)", rand_read_iops, rand_r_lat_us);

        // 5. Small File Creation & Inlining Benchmark
        let num_files = 10_000;
        print!("Running Small File Creation Benchmark ({} files, inlined)... ", num_files);
        let _ = fs.create_dir("/small_files");
        let sample_data = b"TitanFS inline inode payload test 1234567890!";

        let start = Instant::now();
        for i in 0..num_files {
            let path = format!("/small_files/file_{}.txt", i);
            fs.create_file(&path, 0o644).expect("Create small file failed");
            fs.write_file(&path, 0, sample_data).expect("Write small file failed");
        }
        fs.sync().expect("Sync failed");
        let small_create_dur = start.elapsed();
        let small_file_create_per_sec = (num_files as f64) / small_create_dur.as_secs_f64();
        println!("{:.0} files/sec", small_file_create_per_sec);

        // 6. Directory Traversal Benchmark
        print!("Running Directory Traversal Benchmark ({} entries)... ", num_files);
        let start = Instant::now();
        let entries = fs.list_dir("/small_files").expect("List dir failed");
        let dir_dur = start.elapsed();
        let dir_traversal_per_sec = (entries.len() as f64) / dir_dur.as_secs_f64();
        println!("{:.0} entries/sec", dir_traversal_per_sec);

        // 7. Deduplication Benchmark
        print!("Running Deduplication Efficiency Benchmark... ");
        let _ = fs.create_file("/dedup_test.bin", 0o644);
        let duplicate_chunk = [0xCDu8; 4096];
        for i in 0..500 {
            let offset = (i as u64) * 4096;
            fs.write_file("/dedup_test.bin", offset, &duplicate_chunk).expect("Dedup write failed");
        }
        let saved = fs.dedup.saved_blocks();
        let dedup_efficiency_pct = if saved > 0 { (saved as f64 / 500.0) * 100.0 } else { 0.0 };
        println!("{} blocks deduped ({:.1}% space saved)", saved, dedup_efficiency_pct);

        // 8. Host Comparison Benchmark (NTFS / OS disk)
        println!("\nMeasuring Host Filesystem Performance for Side-by-Side Comparison...");
        let host_temp_dir = std::env::temp_dir().join("titanfs_host_bench");
        let _ = fs::create_dir_all(&host_temp_dir);

        // Host sequential write
        let host_file_path = host_temp_dir.join("host_seq.bin");
        let host_write_size = size_mb.min(100);
        let mut host_file = File::create(&host_file_path).expect("Host file create failed");
        let start = Instant::now();
        for _ in 0..host_write_size {
            host_file.write_all(&write_buf).expect("Host write failed");
        }
        host_file.flush().expect("Host flush failed");
        let host_write_dur = start.elapsed();
        let host_seq_write_mb_s = (host_write_size as f64) / host_write_dur.as_secs_f64();

        // Host sequential read
        let mut host_read_file = File::open(&host_file_path).expect("Host open failed");
        let mut r_buf = vec![0u8; 1024 * 1024];
        let start = Instant::now();
        for _ in 0..host_write_size {
            host_read_file.read_exact(&mut r_buf).expect("Host read failed");
        }
        let host_read_dur = start.elapsed();
        let host_seq_read_mb_s = (host_write_size as f64) / host_read_dur.as_secs_f64();

        // Host small file creation
        let host_small_dir = host_temp_dir.join("small");
        let _ = fs::create_dir_all(&host_small_dir);
        let host_small_count = 2000;
        let start = Instant::now();
        for i in 0..host_small_count {
            let p = host_small_dir.join(format!("f_{}.txt", i));
            let mut f = File::create(&p).expect("Host small file create failed");
            f.write_all(sample_data).expect("Host small write failed");
        }
        let host_small_dur = start.elapsed();
        let host_small_file_per_sec = (host_small_count as f64) / host_small_dur.as_secs_f64();

        // Clean up host temp files
        let _ = fs::remove_dir_all(&host_temp_dir);

        let results = BenchmarkResults {
            seq_write_mb_s,
            seq_read_mb_s,
            rand_write_iops,
            rand_read_iops,
            small_file_create_per_sec,
            dir_traversal_per_sec,
            dedup_efficiency_pct,
            host_seq_write_mb_s,
            host_seq_read_mb_s,
            host_small_file_per_sec,
        };

        Self::print_summary_table(&results);
        results
    }

    fn print_summary_table(res: &BenchmarkResults) {
        println!("\n=========================================================================");
        println!("                   TITANFS vs HOST FILESYSTEM COMPARISON");
        println!("=========================================================================");
        println!("{:<32} {:<20} {:<20}", "METRIC", "TITANFS (In-Memory)", "HOST OS (NTFS)");
        println!("{:-<73}", "");
        println!(
            "{:<32} {:<20.2} {:<20.2}",
            "Sequential Write (MB/s)", res.seq_write_mb_s, res.host_seq_write_mb_s
        );
        println!(
            "{:<32} {:<20.2} {:<20.2}",
            "Sequential Read (MB/s)", res.seq_read_mb_s, res.host_seq_read_mb_s
        );
        println!(
            "{:<32} {:<20.0} {:<20.0}",
            "Small File Creation (files/s)", res.small_file_create_per_sec, res.host_small_file_per_sec
        );
        println!("{:<32} {:<20.0} {:<20}", "Random 4KB Write IOPS", res.rand_write_iops, "N/A");
        println!("{:<32} {:<20.0} {:<20}", "Random 4KB Read IOPS", res.rand_read_iops, "N/A");
        println!(
            "{:<32} {:<20.0} {:<20}",
            "Directory Scan (entries/s)", res.dir_traversal_per_sec, "N/A"
        );
        println!(
            "{:<32} {:<20.1} {:<20}",
            "Dedup Space Savings (%)", res.dedup_efficiency_pct, "N/A"
        );
        println!("=========================================================================\n");
    }
}
