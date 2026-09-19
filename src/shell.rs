//! Interactive Virtual Filesystem Shell for TitanFS.

use std::io::{self, BufRead, Write};
use crate::block_dev::BlockDevice;
use crate::fs::FileSystem;

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
        match self.fs.write_file(&abs, 0, data) {
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

    fn print_help(&self) {
        println!("Available Commands:");
        println!("  ls [path]                     - List directory contents");
        println!("  cd <path>                     - Change current directory");
        println!("  pwd                           - Print working directory");
        println!("  mkdir <path>                  - Create a new directory");
        println!("  touch <path>                  - Create an empty file");
        println!("  cat <path>                    - Read and display file contents");
        println!("  write <path> <text...>        - Write text to file");
        println!("  rm <path>                     - Remove a file");
        println!("  rmdir <path>                  - Remove an empty directory");
        println!("  stat <path>                   - Display file/directory metadata");
        println!("  df                            - Display disk space usage and dedup stats");
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
