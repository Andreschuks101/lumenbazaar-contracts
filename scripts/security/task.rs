use std::env;
use std::path::PathBuf;
use std::process::{Command, ExitCode};

fn main() -> ExitCode {
    let task = env::args().nth(1).unwrap_or_else(|| "scan".to_owned());
    let script = match task.as_str() {
        "scan" => "scan_malware.py",
        "test" => "test_scan_malware.py",
        _ => {
            eprintln!("expected security task 'scan' or 'test'");
            return ExitCode::from(2);
        }
    };

    let manifest_directory = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let repository_root = manifest_directory
        .parent()
        .and_then(|path| path.parent())
        .expect("security task manifest must be under scripts/security");
    let script_path = manifest_directory.join(script);

    for interpreter in ["python3", "python"] {
        let available = Command::new(interpreter)
            .arg("--version")
            .output()
            .is_ok_and(|output| output.status.success());
        if !available {
            continue;
        }

        let status = Command::new(interpreter)
            .current_dir(repository_root)
            .env("PYTHONDONTWRITEBYTECODE", "1")
            .arg(&script_path)
            .status();

        match status {
            Ok(status) => {
                return status
                    .code()
                    .map(|code| ExitCode::from(code as u8))
                    .unwrap_or(ExitCode::FAILURE);
            }
            Err(error) => {
                eprintln!("failed to run {interpreter}: {error}");
                return ExitCode::FAILURE;
            }
        }
    }

    eprintln!("Python 3 is required to run the malware scanner");
    ExitCode::from(127)
}
