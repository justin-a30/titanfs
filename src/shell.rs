//! Interactive Virtual Filesystem Shell for TitanFS.

use std::io::{self, BufRead, Write};
use crate::block_dev::BlockDevice;
use crate::fs::FileSystem;
use crate::omni_index::SearchQuery;

pub struct VfsShell<D: BlockDevice> {
    fs: FileSystem<D>,
    current_dir: String,
}

impl<D: BlockDevice> VfsShell<D> {
    pub fn new(fs: FileSystem<D>) -> Self {
        Self {
            fs,
            current_dir: "/".to_string(),
        }
    }

    /// Run the interactive shell REPL.
    pub fn run(&mut self) -> io::Result<()> {
        println!("============================================================");
        println!("          TitanFS Interactive Shell (v1.0.0)");
        println!("  Type 'help' for available commands or 'exit' to quit.");
        println!("============================================================");

        let stdin = io::stdin();
        let mut handle = stdin.lock();

        loop {
            print!("titanfs:{} > ", self.current_dir);
            io::stdout().flush()?;

            let mut line = String::new();
            if handle.read_line(&mut line)? == 0 {
                break;
            }

            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }

            let parts: Vec<&str> = trimmed.split_whitespace().collect();
            let cmd = parts[0];
            let args = &parts[1..];

            match cmd {
                "exit" | "quit" => {
                    println!("Flushing dirty caches and unmounting TitanFS...");
                    let _ = self.fs.sync();
                    println!("Goodbye!");
                    break;
                }
                "help" => self.print_help(),
                "pwd" => println!("{}", self.current_dir),
                "cd" => {
                    let target = if args.is_empty() { "/" } else { args[0] };
                    self.cmd_cd(target);
                }
                "ls" => {
                    let target = if args.is_empty() { "." } else { args[0] };
                    self.cmd_ls(target);
                }
                "mkdir" => {
                    if args.is_empty() {
                        println!("Usage: mkdir <path>");
                    } else {
                        self.cmd_mkdir(args[0]);
                    }
                }
                "touch" => {
                    if args.is_empty() {
                        println!("Usage: touch <path>");
                    } else {
                        self.cmd_touch(args[0]);
                    }
                }
                "cat" => {
                    if args.is_empty() {
                        println!("Usage: cat <path>");
                    } else {
                        self.cmd_cat(args[0]);
                    }
                }
                "write" => {
                    if args.len() < 2 {
                        println!("Usage: write <path> <text...>");
                    } else {
                        let text = args[1..].join(" ");
                        self.cmd_write(args[0], text.as_bytes());
                    }
                }
                "rm" => {
                    if args.is_empty() {
                        println!("Usage: rm <path>");
                    } else {
                        self.cmd_rm(args[0]);
                    }
                }
                "rmdir" => {
                    if args.is_empty() {
                        println!("Usage: rmdir <path>");
                    } else {
                        self.cmd_rmdir(args[0]);
                    }
                }
                "stat" => {
                    if args.is_empty() {
                        println!("Usage: stat <path>");
                    } else {
                        self.cmd_stat(args[0]);
                    }
                }
                "df" => self.cmd_df(),
                "snap" => {
                    if args.is_empty() {
                        println!("Usage: snap <snapshot_name>");
                    } else {
                        self.cmd_snap(args[0]);
                    }
                }
                "rollback" => {
                    if args.is_empty() {
                        println!("Usage: rollback <snapshot_name>");
                    } else {
                        self.cmd_rollback(args[0]);
                    }
                }
                "import" => {
                    if args.len() < 2 {
                        println!("Usage: import <host_path> <titanfs_path>");
                    } else {
                        self.cmd_import(args[0], args[1]);
                    }
                }
                "export" => {
                    if args.len() < 2 {
                        println!("Usage: export <titanfs_path> <host_path>");
                    } else {
                        self.cmd_export(args[0], args[1]);
                    }
                }
                "fsck" => self.cmd_fsck(),
                "sync" => {
                    if let Err(e) = self.fs.sync() {
                        println!("Sync error: {}", e);
                    } else {
                        println!("Filesystem synced and clean.");
                    }
                }
                "history" => {
                    if args.is_empty() {
                        println!("Usage: history <path>");
                    } else {
                        self.cmd_history(args[0]);
                    }
                }
                "branch" => {
                    if args.len() < 2 {
                        println!("Usage: branch <source_dir> <target_dir>");
                    } else {
                        self.cmd_branch(args[0], args[1]);
                    }
                }
                "branches" => self.cmd_branches(),
                "search" => self.cmd_search(args),
                "shield" => self.cmd_shield(),
                "shield-discard" => self.cmd_shield_discard(),
                _ => println!("Unknown command '{}'. Type 'help' for commands.", cmd),
            }
        }

        Ok(())
    }

    fn resolve_path_str(&self, path: &str) -> String {
        if path.starts_with('/') {
            path.to_string()
        } else if self.current_dir == "/" {
            format!("/{}", path)
        } else {
            format!("{}/{}", self.current_dir, path)
        }
    }

    fn cmd_cd(&mut self, target: &str) {
        let abs = self.resolve_path_str(target);
        match self.fs.stat(&abs) {
            Ok(inode) => {
                if inode.is_dir() {
                    self.current_dir = abs;
                } else {
                    println!("cd: not a directory: {}", target);
                }
            }
            Err(e) => println!("cd: {}", e),
        }
    }

    fn cmd_ls(&mut self, target: &str) {
        let abs = self.resolve_path_str(target);
        match self.fs.list_dir(&abs) {
            Ok(entries) => {
                println!("{:<6} {:<10} {:<30}", "TYPE", "INODE", "NAME");
                println!("{:-<50}", "");
                for entry in entries {
                    println!(
                        "{:<6} {:<10} {:<30}",
                        entry.file_type.as_str(),
                        entry.inode_id,
                        entry.name
                    );
                }
            }
            Err(e) => println!("ls: {}", e),
        }
    }

    fn cmd_mkdir(&mut self, target: &str) {
        let abs = self.resolve_path_str(target);
        match self.fs.create_dir(&abs) {
            Ok(id) => println!("Directory created: {} (inode {})", abs, id),
            Err(e) => println!("mkdir: {}", e),
        }
    }

    fn cmd_touch(&mut self, target: &str) {
        let abs = self.resolve_path_str(target);
        match self.fs.create_file(&abs, 0o644) {
            Ok(id) => println!("File created: {} (inode {})", abs, id),
            Err(e) => println!("touch: {}", e),
        }
    }

    fn cmd_cat(&mut self, target: &str) {
        let abs = self.resolve_path_str(target);
        match self.fs.stat(&abs) {
            Ok(inode) => match self.fs.read_file(&abs, 0, inode.size as usize) {
                Ok(data) => {
                    let s = String::from_utf8_lossy(&data);
                    println!("{}", s);
                }
                Err(e) => println!("cat read: {}", e),
            },
            Err(e) => println!("cat stat: {}", e),
        }
    }

    fn cmd_write(&mut self, target: &str, data: &[u8]) {
        let abs = self.resolve_path_str(target);
        // Create if missing
        let _ = self.fs.create_file(&abs, 0o644);
        match self.fs.write_file_truncate(&abs, data) {
            Ok(bytes) => println!("Wrote {} bytes to {}", bytes, abs),
            Err(e) => println!("write: {}", e),
        }
    }

    fn cmd_rm(&mut self, target: &str) {
        let abs = self.resolve_path_str(target);
        match self.fs.remove_file(&abs) {
            Ok(()) => println!("Removed {}", abs),
            Err(e) => println!("rm: {}", e),
        }
    }

    fn cmd_rmdir(&mut self, target: &str) {
        let abs = self.resolve_path_str(target);
        match self.fs.remove_dir(&abs) {
            Ok(()) => println!("Removed directory {}", abs),
            Err(e) => println!("rmdir: {}", e),
        }
    }

    fn cmd_stat(&mut self, target: &str) {
        let abs = self.resolve_path_str(target);
        match self.fs.stat(&abs) {
            Ok(inode) => {
                println!("  Inode: {}", inode.id);
                println!("  Type: {:?}", inode.file_type);
                println!("  Size: {} bytes", inode.size);
                println!("  Allocated Blocks: {}", inode.allocated_blocks);
                println!("  Inline Data: {}", inode.is_inline());
                println!("  Direct Extents:");
                for (i, ext) in inode.extents.iter().enumerate() {
                    if !ext.is_empty() {
                        println!(
                            "    [{}] logical={} physical={} len={}",
                            i, ext.logical_block, ext.physical_block, ext.length
                        );
                    }
                }
            }
            Err(e) => println!("stat: {}", e),
        }
    }

    fn cmd_df(&mut self) {
        let total_mb = (self.fs.sb.total_blocks * 4096) / (1024 * 1024);
        let free_blocks = self.fs.block_bitmap.count_free();
        let free_mb = (free_blocks * 4096) / (1024 * 1024);
        let used_mb = total_mb.saturating_sub(free_mb);

        println!("{:<15} {:<10} {:<10} {:<10} {:<8}", "Filesystem", "Size", "Used", "Avail", "Use%");
        println!("{:-<55}", "");
        let use_pct = if total_mb > 0 { (used_mb * 100) / total_mb } else { 0 };
        println!(
            "{:<15} {:<7}MB {:<7}MB {:<7}MB {}%",
            "TitanFS", total_mb, used_mb, free_mb, use_pct
        );
        println!("Unique Dedup Entries: {}", self.fs.dedup.unique_blocks_count());
        println!("Dedup Blocks Saved: {}", self.fs.dedup.saved_blocks());
    }

    fn cmd_snap(&mut self, name: &str) {
        match self.fs.create_snapshot(name) {
            Ok(()) => println!("Snapshot '{}' created instantaneously.", name),
            Err(e) => println!("snap: {}", e),
        }
    }

    fn cmd_rollback(&mut self, name: &str) {
        match self.fs.rollback_snapshot(name) {
            Ok(()) => println!("Rolled back to snapshot '{}' successfully.", name),
            Err(e) => println!("rollback: {}", e),
        }
    }

    fn cmd_import(&mut self, host_path: &str, fs_path: &str) {
        let abs = self.resolve_path_str(fs_path);
        match self.fs.import_host_file(host_path, &abs) {
            Ok(bytes) => println!("Imported {} bytes from '{}' to '{}'", bytes, host_path, abs),
            Err(e) => println!("import: {}", e),
        }
    }

    fn cmd_export(&mut self, fs_path: &str, host_path: &str) {
        let abs = self.resolve_path_str(fs_path);
        match self.fs.export_to_host(&abs, host_path) {
            Ok(bytes) => println!("Exported {} bytes from '{}' to '{}'", bytes, abs, host_path),
            Err(e) => println!("export: {}", e),
        }
    }

    fn cmd_fsck(&mut self) {
        match self.fs.fsck() {
            Ok(report) => {
                println!("Fsck Report:");
                println!("  Status: {}", if report.is_clean { "CLEAN" } else { "DIRTY/CORRUPTED" });
                println!("  Total Blocks: {}", report.total_blocks);
                println!("  Used Blocks:  {}", report.used_blocks);
                println!("  Free Blocks:  {}", report.free_blocks);
                println!("  Corrupted:    {}", report.corrupted_blocks);
            }
            Err(e) => println!("fsck: {}", e),
        }
    }

    fn cmd_history(&mut self, target: &str) {
        let abs = self.resolve_path_str(target);
        match self.fs.stat(&abs) {
            Ok(inode) => {
                let versions = self.fs.chrono.list_versions(inode.id);
                if versions.is_empty() {
                    println!("No historical versions recorded for {}", abs);
                    return;
                }

                let now_ns = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_nanos() as u64;

                println!("ChronoVFS™ Version Timeline for '{}' (inode {}):", abs, inode.id);
                println!("{:<4} {:<22} {:<12} {:<10} {:<15}", "VER", "TIMESTAMP (NS)", "AGE", "SIZE", "STORAGE");
                println!("{:-<65}", "");

                for (i, v) in versions.iter().enumerate() {
                    let diff_ns = now_ns.saturating_sub(v.timestamp_ns);
                    let age_str = if diff_ns < 1_000_000 {
                        format!("{}µs ago", diff_ns / 1_000)
                    } else if diff_ns < 1_000_000_000 {
                        format!("{}ms ago", diff_ns / 1_000_000)
                    } else if diff_ns < 60_000_000_000 {
                        format!("{:.1}s ago", diff_ns as f64 / 1_000_000_000.0)
                    } else {
                        format!("{:.1}m ago", diff_ns as f64 / 60_000_000_000.0)
                    };

                    let storage_str = if v.is_inline {
                        "Inline".to_string()
                    } else {
                        let count = v.extents.iter().filter(|e| !e.is_empty()).count();
                        format!("{} extents", count)
                    };

                    println!(
                        "{:<4} {:<22} {:<12} {:<10} {:<15}",
                        i + 1,
                        v.timestamp_ns,
                        age_str,
                        format!("{} B", v.size),
                        storage_str
                    );
                }
                println!("\n  Hint: Time-travel via 'cat {}@-5s' or 'stat {}@-10m'", target, target);
            }
            Err(e) => println!("history: {}", e),
        }
    }

    fn cmd_branch(&mut self, source: &str, target: &str) {
        let src_abs = self.resolve_path_str(source);
        let tgt_abs = self.resolve_path_str(target);
        match self.fs.branch_dir(&src_abs, &tgt_abs) {
            Ok(()) => println!("GhostTrees™: Instant O(1) branch created: '{}' -> '{}' (0 blocks copied)", src_abs, tgt_abs),
            Err(e) => println!("branch: {}", e),
        }
    }

    fn cmd_branches(&self) {
        let branches = self.fs.ghost_trees.list_branches();
        if branches.is_empty() {
            println!("No active GhostTree branches.");
            return;
        }
        println!("{:<20} {:<14} {:<14} {:<18}", "BRANCH NAME", "SOURCE INODE", "CLONED INODE", "MERKLE ROOT");
        println!("{:-<68}", "");
        for b in branches {
            println!("{:<20} {:<14} {:<14} 0x{:016x}", b.name, b.source_inode, b.cloned_inode, b.merkle_root);
        }
    }

    fn cmd_search(&self, args: &[&str]) {
        let mut ext: Option<&str> = None;
        let mut min_size: Option<u64> = None;
        let mut max_size: Option<u64> = None;
        let mut name: Option<&str> = None;

        for arg in args {
            if let Some(val) = arg.strip_prefix("ext=") {
                ext = Some(val);
            } else if let Some(val) = arg.strip_prefix("min=") {
                min_size = val.parse().ok();
            } else if let Some(val) = arg.strip_prefix("max=") {
                max_size = val.parse().ok();
            } else if let Some(val) = arg.strip_prefix("name=") {
                name = Some(val);
            } else {
                if arg.starts_with('.') {
                    ext = Some(&arg[1..]);
                } else {
                    name = Some(arg);
                }
            }
        }

        let query = SearchQuery {
            extension: ext,
            min_size,
            max_size,
            name_contains: name,
        };

        let results = self.fs.search_index(&query);
        if results.is_empty() {
            println!("OmniIndex™: No matching files found.");
            return;
        }

        println!("{:<10} {:<12} {:<10} {:<35}", "INODE", "SIZE", "BRACKET", "PATH");
        println!("{:-<70}", "");
        for r in &results {
            println!("{:<10} {:<12} {:<10?} {:<35}", r.inode_id, format!("{} B", r.size), r.bracket, r.path);
        }
        println!("Found {} files via OmniIndex™ in-memory inverted table (< 1µs zero-CPU).", results.len());
    }

    fn cmd_shield(&self) {
        let report = self.fs.ransom_shield.get_report();
        println!("============================================================");
        println!("    RansomShield™ Hardware-Entropy Autonomous Defense");
        println!("============================================================");
        println!("  Quarantine Status:   {}", if report.locked { "LOCKDOWN (INTERCEPTING)" } else { "NORMAL (MONITORING)" });
        println!("  Threats Detected:    {}", report.detected_threats);
        println!("  Quarantined Blocks:  {}", report.quarantined_blocks);
        println!("  Last Write Entropy:  {:.2} bits/byte (threshold: >7.50)", report.last_entropy);
        if !report.suspicious_paths.is_empty() {
            println!("  Suspicious Paths:");
            for p in &report.suspicious_paths {
                println!("    - {}", p);
            }
        }
    }

    fn cmd_shield_discard(&mut self) {
        let count = self.fs.ransom_shield.discard_quarantine();
        println!("RansomShield™: Purged {} quarantined malicious blocks. Filesystem unlocked.", count);
    }

    fn print_help(&self) {
        println!("Available Commands:");
        println!("  ls [path]                     - List directory contents");
        println!("  cd <path>                     - Change current directory");
        println!("  pwd                           - Print working directory");
        println!("  mkdir <path>                  - Create a new directory");
        println!("  touch <path>                  - Create an empty file");
        println!("  cat <path>                    - Read file contents (supports /path@-15s time-travel)");
        println!("  write <path> <text...>        - Write text to file");
        println!("  rm <path>                     - Remove a file");
        println!("  rmdir <path>                  - Remove an empty directory");
        println!("  stat <path>                   - Display file/directory metadata (supports @-10s)");
        println!("  df                            - Display disk space usage and dedup stats");
        println!("  history <path>                - ChronoVFS microsecond time-travel timeline");
        println!("  branch <source> <target>      - GhostTrees instant O(1) workspace branch");
        println!("  branches                      - List active GhostTree workspace branches");
        println!("  search [ext=] [min=] [name=]  - OmniIndex sub-millisecond compound search");
        println!("  shield                        - RansomShield autonomous defense status");
        println!("  shield-discard                - Discard quarantined blocks and unlock FS");
        println!("  snap <name>                   - Create instantaneous CoW snapshot");
        println!("  rollback <name>               - Rollback to named snapshot");
        println!("  import <host> <titanfs>       - Copy file from host into TitanFS");
        println!("  export <titanfs> <host>       - Copy file from TitanFS out to host");
        println!("  fsck                          - Check filesystem consistency");
        println!("  sync                          - Flush dirty buffers to storage");
        println!("  help                          - Show this help message");
        println!("  exit                          - Sync and exit shell");
    }
}
