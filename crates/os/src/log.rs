//! Journal sur la carte SD, plafonné pour ne jamais remplir la carte.

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::Path;
use std::sync::Mutex;

const MAX_LOG_BYTES: u64 = 256 * 1024;

static LOG: Mutex<Option<File>> = Mutex::new(None);

/// Ouvre le journal ; il repart de zéro s'il dépasse 256 Ko.
pub fn init(path: &Path) {
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let too_big = std::fs::metadata(path).is_ok_and(|m| m.len() > MAX_LOG_BYTES);
    let file = OpenOptions::new()
        .create(true)
        .append(!too_big)
        .write(true)
        .truncate(too_big)
        .open(path)
        .ok();
    *LOG.lock().unwrap_or_else(|e| e.into_inner()) = file;
}

pub fn write(message: std::fmt::Arguments) {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let line = format!("[{secs}] {message}\n");
    eprint!("{line}");
    if let Some(file) = LOG.lock().unwrap_or_else(|e| e.into_inner()).as_mut() {
        let _ = file.write_all(line.as_bytes());
    }
}

#[macro_export]
macro_rules! log {
    ($($arg:tt)*) => {
        $crate::log::write(format_args!($($arg)*))
    };
}
