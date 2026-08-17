//! The SingleStepTests oracle sets the chip suites run against.

use std::path::{Path, PathBuf};
use std::process::Command;

/// Sparse-clone `url` into `root`, check out `paths`, and record the commit
/// fetched. `env_var` names the override a caller can point at an existing
/// checkout instead, so the failure message can offer both routes.
pub fn fetch_oracle(root: &Path, url: &str, paths: &[&str], env_var: &str) {
    let git = |args: &[&str], cwd: Option<&Path>| {
        let mut command = Command::new("git");
        command.args(args);
        if let Some(cwd) = cwd {
            command.current_dir(cwd);
        }
        let ran = command.status().map(|status| status.success());
        assert!(
            ran.unwrap_or(false),
            "git {} failed — clone {url} into {} (sparse: {}), or set {env_var}",
            args.join(" "),
            root.display(),
            paths.join(" "),
        );
    };

    if !root.is_dir() {
        git(
            &[
                "clone",
                "--depth",
                "1",
                "--filter=blob:none",
                "--sparse",
                url,
                &root.display().to_string(),
            ],
            None,
        );
    }

    let mut sparse = vec!["sparse-checkout", "set"];
    sparse.extend_from_slice(paths);
    git(&sparse, Some(root));
    git(&["checkout", "--", "."], Some(root));

    let head = Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(root)
        .output()
        .expect("git rev-parse");
    std::fs::write(root.join("FETCHED_COMMIT"), head.stdout).expect("record fetched commit");
}

/// Run every oracle file in turn, requiring each case to pass and the sweep as
/// a whole to have run more than `floor` cases (a data set that silently
/// failed to fetch would otherwise pass vacuously).
///
/// `cases` parses one file's bytes; `run` reports a failing case, naming it in
/// the terms the report should carry.
pub fn assert_oracle_sweep<C>(
    files: &[PathBuf],
    floor: usize,
    cases: impl Fn(&[u8]) -> Vec<C>,
    run: impl Fn(&C) -> Result<(), String>,
) {
    let mut total_passed = 0usize;
    let mut total_failed = 0usize;
    let mut bad_files = Vec::new();

    for path in files {
        let raw = std::fs::read(path).expect("readable test file");
        let mut failed = 0usize;
        let mut examples = Vec::new();
        for case in &cases(&raw) {
            match run(case) {
                Ok(()) => total_passed += 1,
                Err(problem) => {
                    failed += 1;
                    if examples.len() < 3 {
                        examples.push(format!("  {problem}"));
                    }
                }
            }
        }
        total_failed += failed;
        if failed > 0 {
            let name = path.file_stem().unwrap().to_string_lossy();
            bad_files.push(format!("{name}: {failed} failed\n{}", examples.join("\n")));
        }
    }

    assert!(
        bad_files.is_empty(),
        "{} opcode files with failures ({total_passed} passed, {total_failed} failed):\n{}",
        bad_files.len(),
        bad_files.join("\n")
    );
    assert!(
        total_passed > floor,
        "suspiciously few cases ran: {total_passed}"
    );
}
