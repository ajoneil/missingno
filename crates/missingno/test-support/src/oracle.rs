//! The SingleStepTests oracle sets the chip suites run against.

use std::path::Path;
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
