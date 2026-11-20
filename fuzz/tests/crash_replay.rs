use lumenbazaar_fuzz::{parse_hex_seed, run_reporting, TARGETS};
use std::{fs, path::PathBuf};

#[test]
fn reviewed_crash_artifacts_replay_with_fixed_outcomes() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("crashes");
    for (name, target) in TARGETS {
        let directory = root.join(name);
        if !directory.exists() {
            continue;
        }
        let mut files: Vec<_> = fs::read_dir(&directory)
            .expect("read crash artifact directory")
            .map(|entry| entry.expect("crash artifact entry").path())
            .filter(|path| path.extension().is_some_and(|value| value == "hex"))
            .collect();
        files.sort();
        for path in files {
            let text = fs::read_to_string(&path).expect("reviewed artifact is UTF-8");
            let bytes = parse_hex_seed(&text).expect("reviewed artifact is valid hex");
            assert!(
                bytes.len() <= 4096,
                "reviewed crash input exceeds replay memory bound"
            );
            run_reporting(name, *target, &bytes);
        }
    }
}
