use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::thread;

pub use libdatamile::harvest::harvest_file;
use libdatamile::harvest::{SUPPORTED_AUDIO_EXTENSIONS, is_audio_file};
use rayon::prelude::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FormatMode {
    Compact,
    Pretty,
}

pub fn run(roots: Vec<PathBuf>, format: FormatMode) {
    let mut files = Vec::new();

    for root in roots {
        files.extend(scan_files(&root, SUPPORTED_AUDIO_EXTENSIONS));
    }

    if files.is_empty() {
        return;
    }

    let (tx, rx) = mpsc::channel::<String>();

    let printer_handle = thread::spawn(move || {
        let stdout = io::stdout();
        let mut handle = io::BufWriter::new(stdout.lock());
        for line in rx {
            writeln!(handle, "{line}").ok();
        }
    });

    files.par_iter().for_each_with(tx, |tx, path| {
        if let Ok(payload) = harvest_file(path) {
            let json_res = match format {
                FormatMode::Pretty => serde_json::to_string_pretty(&payload),
                FormatMode::Compact => serde_json::to_string(&payload),
            };

            if let Ok(json) = json_res {
                tx.send(json).ok();
            }
        }
    });

    printer_handle
        .join()
        .expect("harvester printer thread panicked");
}

fn scan_files(root: &Path, extensions: &[&str]) -> Vec<PathBuf> {
    if root.is_file() {
        if is_audio_file(root, extensions) {
            return vec![root.to_path_buf()];
        }
        return Vec::new();
    }

    libdatamile::scanner::scan_audio_files(root, extensions)
}
