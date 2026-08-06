//! Gate G4 — recorded, not gating. The real benchmark gate lands with the release
//! pipeline; this exists so a change that makes loading ten times slower is noticed.
//!
//! Thresholds are deliberately far above the budget (cold < 30 ms, warm < 2 ms) because
//! a shared CI runner is not a quiet machine. The measured numbers are written to
//! `target/g4-timings.txt` for the record.

use std::fs;
use std::time::{Duration, Instant};

use rune_config::env::Environment;
use rune_config::load::load;
use tempfile::TempDir;

fn repo() -> TempDir {
  let dir = tempfile::tempdir().expect("create tempdir");
  fs::create_dir_all(dir.path().join(".git")).expect("create .git");
  fs::write(dir.path().join(".git/HEAD"), "ref: refs/heads/main\n").expect("write HEAD");
  fs::write(dir.path().join("helper.ts"), "export const PORT: number = 4000;\n").expect("helper");
  fs::write(
    dir.path().join("rune.config.ts"),
    "import { PORT } from './helper';\n\
     export default { scripts: { dev: { command: `vite --port ${PORT}` } } };\n",
  )
  .expect("config");
  dir
}

fn measure(dir: &TempDir) -> Duration {
  let start = Instant::now();
  load(dir.path(), &Environment::default()).expect("config loads");
  start.elapsed()
}

#[test]
fn cold_and_warm_load_are_recorded() {
  let dir = repo();

  let cold = measure(&dir);
  // Three warm runs, best of, so one scheduling hiccup does not define the number.
  let warm = (0..3).map(|_| measure(&dir)).min().expect("three samples");

  let record = format!(
    "cold: {:.3} ms (budget 30 ms)\nwarm: {:.3} ms (budget 2 ms)\n",
    cold.as_secs_f64() * 1000.0,
    warm.as_secs_f64() * 1000.0
  );
  let _ = fs::write("../../target/g4-timings.txt", &record);

  assert!(cold < Duration::from_millis(300), "cold load regressed badly\n{record}");
  assert!(warm < Duration::from_millis(50), "warm load regressed badly\n{record}");
  assert!(warm <= cold, "the cache made loading slower\n{record}");
}
