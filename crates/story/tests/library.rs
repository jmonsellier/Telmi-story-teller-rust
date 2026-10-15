//! Vérifie le moteur sur une vraie bibliothèque Telmi.
//!
//! `TELMI_STORIES=~/.telmi/stories cargo test --release -p telmi-story -- --nocapture`
//! Sans la variable, le test ne fait rien.

use std::alloc::{GlobalAlloc, Layout, System};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;

/// Compte les octets alloués pour mesurer le pic pendant la lecture.
struct Counting;

static CURRENT: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let now = CURRENT.fetch_add(layout.size(), Ordering::Relaxed) + layout.size();
        PEAK.fetch_max(now, Ordering::Relaxed);
        // SAFETY: délégation directe à l'allocateur système.
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        CURRENT.fetch_sub(layout.size(), Ordering::Relaxed);
        // SAFETY: délégation directe à l'allocateur système.
        unsafe { System.dealloc(ptr, layout) }
    }
}

#[global_allocator]
static ALLOCATOR: Counting = Counting;

use telmi_story::{load_story, Player, Step};

fn library() -> Option<PathBuf> {
    std::env::var_os("TELMI_STORIES").map(PathBuf::from)
}

fn story_dirs(root: &Path) -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = std::fs::read_dir(root)
        .expect("dossier des histoires lisible")
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.join("nodes.json").is_file())
        .collect();
    dirs.sort();
    dirs
}

#[test]
fn every_story_loads_with_bounded_memory() {
    let Some(root) = library() else { return };
    for dir in story_dirs(&root) {
        let nodes = dir.join("nodes.json");
        let size = std::fs::metadata(&nodes).unwrap().len();
        let base = CURRENT.load(Ordering::Relaxed);
        PEAK.store(base, Ordering::Relaxed);
        let started = Instant::now();
        let story = load_story(&nodes).unwrap_or_else(|e| panic!("{}: {e}", dir.display()));
        let elapsed = started.elapsed();
        let heap = story.heap_bytes();
        let retained = CURRENT.load(Ordering::Relaxed) - base;
        let peak = PEAK.load(Ordering::Relaxed) - base;
        println!(
            "{:40} {:5.1} Mo json → gardé {:5.2} Mo, pic {:5.2} Mo, {:6} étapes, {:4} ms",
            dir.file_name().unwrap().to_string_lossy(),
            size as f64 / 1e6,
            retained as f64 / 1e6,
            peak as f64 / 1e6,
            story.stage_count(),
            elapsed.as_millis()
        );
        assert!(peak < 16_000_000, "{}: pic de {peak} octets", dir.display());
        assert!(heap < 12_000_000, "{}: {heap} octets", dir.display());

        let mut missing = 0;
        for id in 0..story.stage_count() as u32 {
            let stage = story.stage(id).unwrap();
            if let Some(a) = stage.audio {
                missing += usize::from(!dir.join("audios").join(story.str(a)).is_file());
            }
            if let Some(i) = stage.image {
                missing += usize::from(!dir.join("images").join(story.str(i)).is_file());
            }
        }
        assert_eq!(missing, 0, "{}: fichiers manquants", dir.display());
    }
}

#[test]
fn random_walks_never_panic() {
    let Some(root) = library() else { return };
    for dir in story_dirs(&root) {
        let story = load_story(&dir.join("nodes.json")).unwrap();
        let mut player = Player::new(story, 7);
        let mut seed: u64 = 0x9E37_79B9_7F4A_7C15;
        let mut resets = 0;
        let mut step = player.start(None, 0);
        for t in 0..20_000i64 {
            if step == Step::Reset {
                resets += 1;
                step = player.start(None, t);
                continue;
            }
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            step = match seed % 10 {
                0..=4 => player.ok(t),
                5..=6 => player.select(1, t),
                7 => player.select(-1, t),
                8 => player.home(t),
                _ => {
                    let saved = player.snapshot(t);
                    player.start(Some(&saved), t)
                }
            };
        }
        println!("{:40} {resets} retours au titre", dir.file_name().unwrap().to_string_lossy());
    }
}
