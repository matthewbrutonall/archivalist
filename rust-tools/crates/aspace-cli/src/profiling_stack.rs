use anyhow::{bail, Context, Result};
use std::path::{Path, PathBuf};
use std::process::Command;

/// Locates the repository verifier script from a manifest directory if anchored to `rust-tools/crates/...`.
pub fn verifier_from_manifest_dir(manifest_dir: &Path) -> Option<PathBuf> {
    let crates_dir = manifest_dir.parent()?;
    if crates_dir.file_name().and_then(|n| n.to_str()) != Some("crates") {
        return None;
    }
    let rust_tools = crates_dir.parent()?;
    if rust_tools.file_name().and_then(|n| n.to_str()) != Some("rust-tools") {
        return None;
    }
    Some(rust_tools.join("scripts").join("profiling_stack.py"))
}

/// Evaluates the executable's path ancestry and returns the repository verifier script path
/// ONLY if the executable is located within `rust-tools/target/{debug,release}` (or cargo target triple).
pub fn verifier_from_exe_ancestry(exe_path: &Path) -> Option<PathBuf> {
    let mut current = exe_path.parent()?;

    // If binary is in deps/ (e.g. test runner), step up to the profile dir
    if current.file_name().and_then(|n| n.to_str()) == Some("deps") {
        current = current.parent()?;
    }

    let profile_name = current.file_name().and_then(|n| n.to_str())?;
    if profile_name != "debug" && profile_name != "release" {
        return None;
    }

    let target_dir = current.parent()?;
    let rust_tools_dir = if target_dir.file_name().and_then(|n| n.to_str()) == Some("target") {
        target_dir.parent()?
    } else {
        // Target triple dir, e.g. target/x86_64-unknown-linux-gnu/debug
        let parent_target = target_dir.parent()?;
        if parent_target.file_name().and_then(|n| n.to_str()) == Some("target") {
            parent_target.parent()?
        } else {
            return None;
        }
    };

    if rust_tools_dir.file_name().and_then(|n| n.to_str()) != Some("rust-tools") {
        return None;
    }

    Some(rust_tools_dir.join("scripts").join("profiling_stack.py"))
}

/// Returns candidate verifier paths given a manifest directory and an optional executable path.
/// Only paths anchored to the manifest location or rust-tools/target/{debug,release} ancestry
/// are included. CWD and PATH are never considered.
pub fn candidate_verifier_paths_from(manifest_dir: &Path, exe_path: Option<&Path>) -> Vec<PathBuf> {
    let mut candidates = Vec::new();

    // 1. Compile-time manifest location (CARGO_MANIFEST_DIR is .../rust-tools/crates/aspace-cli)
    if let Some(candidate) = verifier_from_manifest_dir(manifest_dir) {
        candidates.push(candidate);
    }

    // 2. Relative to the running executable anchored to rust-tools/target/{debug,release} ancestry
    if let Some(exe) = exe_path {
        if let Some(candidate) = verifier_from_exe_ancestry(exe) {
            candidates.push(candidate);
        }
    }

    candidates
}

/// Returns candidate verifier paths in the current execution environment.
pub fn candidate_verifier_paths() -> Vec<PathBuf> {
    candidate_verifier_paths_from(
        Path::new(env!("CARGO_MANIFEST_DIR")),
        std::env::current_exe().ok().as_deref(),
    )
}

/// Locates the repository's own `rust-tools/scripts/profiling_stack.py`.
/// The path is NOT user-provided and NEVER falls back to CWD or PATH.
pub fn find_profiling_stack_verifier() -> Result<PathBuf> {
    let candidates = candidate_verifier_paths();
    find_verifier_in_candidates(&candidates)
}

/// Evaluates candidate paths and returns the first existing regular file canonicalized.
pub fn find_verifier_in_candidates(candidates: &[PathBuf]) -> Result<PathBuf> {
    for candidate in candidates {
        if candidate.is_file() {
            if let Ok(canonical) = candidate.canonicalize() {
                if canonical.is_file() {
                    return Ok(canonical);
                }
            }
            return Ok(candidate.clone());
        }
    }
    bail!("profiling-stack verifier script (profiling_stack.py) not found in repository");
}

/// Default verifier that invokes the repository's profiling_stack.py via subprocess argument arrays.
pub fn verify_profiling_stack(state_path: &Path, backend_url: &str, solr_url: &str) -> Result<()> {
    verify_profiling_stack_custom(state_path, backend_url, solr_url, run_verifier_subprocess)
}

/// Verifies profiling stack identity using a pluggable runner for testing.
pub fn verify_profiling_stack_custom<F>(
    state_path: &Path,
    backend_url: &str,
    solr_url: &str,
    runner: F,
) -> Result<()>
where
    F: FnOnce(&Path, &Path, &str, &str) -> Result<()>,
{
    if !state_path.exists() {
        bail!(
            "profiling-stack state file does not exist: {}",
            state_path.display()
        );
    }
    if !state_path.is_file() {
        bail!(
            "profiling-stack state path is not a regular file: {}",
            state_path.display()
        );
    }

    let verifier_script = find_profiling_stack_verifier()?;
    runner(&verifier_script, state_path, backend_url, solr_url)
}

/// Executes `python3 <verifier_script> verify --state <state_path> --backend-url <backend_url> --solr-url <solr_url>`
/// using subprocess argument arrays (no shell).
pub fn run_verifier_subprocess(
    verifier_script: &Path,
    state_path: &Path,
    backend_url: &str,
    solr_url: &str,
) -> Result<()> {
    let output = Command::new("python3")
        .arg(verifier_script)
        .arg("verify")
        .arg("--state")
        .arg(state_path)
        .arg("--backend-url")
        .arg(backend_url)
        .arg("--solr-url")
        .arg(solr_url)
        .output()
        .with_context(|| {
            format!(
                "Failed to execute profiling-stack verifier: python3 {}",
                verifier_script.display()
            )
        })?;

    if !output.status.success() {
        let code = output.status.code().unwrap_or(-1);
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!(
            "Profiling stack verification failed (exit code {}):\nstdout: {}\nstderr: {}",
            code,
            stdout.trim(),
            stderr.trim()
        );
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;

    #[test]
    fn test_find_verifier_in_candidates_absent_fails_closed() {
        let nonexistent = vec![
            PathBuf::from("/nonexistent/path/profiling_stack.py"),
            PathBuf::from("/another/fake/path/profiling_stack.py"),
        ];
        let err = find_verifier_in_candidates(&nonexistent).unwrap_err();
        assert!(err
            .to_string()
            .contains("profiling-stack verifier script (profiling_stack.py) not found"));
    }

    #[test]
    fn test_find_profiling_stack_verifier_finds_repo_script() {
        let script = find_profiling_stack_verifier().expect("should find repo profiling_stack.py");
        assert!(script.is_file());
        assert!(script.ends_with("profiling_stack.py"));
    }

    #[test]
    fn test_candidate_paths_do_not_consider_cwd() {
        let temp_dir = std::env::temp_dir().join(format!(
            "test_cwd_isolation_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let decoy_scripts = temp_dir.join("scripts");
        let decoy_rust_tools_scripts = temp_dir.join("rust-tools").join("scripts");
        fs::create_dir_all(&decoy_scripts).unwrap();
        fs::create_dir_all(&decoy_rust_tools_scripts).unwrap();
        fs::write(decoy_scripts.join("profiling_stack.py"), b"print('fake')").unwrap();
        fs::write(
            decoy_rust_tools_scripts.join("profiling_stack.py"),
            b"print('fake')",
        )
        .unwrap();

        let candidates = candidate_verifier_paths();
        for candidate in &candidates {
            assert!(
                !candidate.starts_with(&temp_dir),
                "Candidate path {:?} should not be derived from temporary directory/CWD",
                candidate
            );
        }

        // Static candidate-path test: verify that arbitrary manifest/exe paths yield zero candidates
        let arbitrary_manifest = temp_dir.join("some").join("arbitrary").join("path");
        let arbitrary_exe = temp_dir.join("bin").join("aspace");
        let arbitrary_candidates =
            candidate_verifier_paths_from(&arbitrary_manifest, Some(&arbitrary_exe));
        assert!(
            arbitrary_candidates.is_empty(),
            "Arbitrary paths outside rust-tools/target/{{debug,release}} must yield no candidates"
        );

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_verifier_from_exe_ancestry_accepts_target_debug_release_only() {
        let root = Path::new("/workspace/checkout/rust-tools");
        let debug_exe = root.join("target").join("debug").join("aspace");
        let release_exe = root.join("target").join("release").join("aspace");
        let triple_exe = root
            .join("target")
            .join("x86_64-unknown-linux-gnu")
            .join("release")
            .join("aspace");
        let deps_exe = root
            .join("target")
            .join("debug")
            .join("deps")
            .join("aspace_cli-12345");

        let expected = root.join("scripts").join("profiling_stack.py");
        assert_eq!(
            verifier_from_exe_ancestry(&debug_exe),
            Some(expected.clone())
        );
        assert_eq!(
            verifier_from_exe_ancestry(&release_exe),
            Some(expected.clone())
        );
        assert_eq!(
            verifier_from_exe_ancestry(&triple_exe),
            Some(expected.clone())
        );
        assert_eq!(verifier_from_exe_ancestry(&deps_exe), Some(expected));

        // Rejections:
        let copied_bin = Path::new("/usr/local/bin/aspace");
        assert_eq!(verifier_from_exe_ancestry(copied_bin), None);

        let cwd_bin = Path::new("/tmp/test/aspace");
        assert_eq!(verifier_from_exe_ancestry(cwd_bin), None);

        let fake_target = Path::new("/home/user/other-project/target/debug/aspace");
        assert_eq!(verifier_from_exe_ancestry(fake_target), None);
    }

    #[test]
    fn test_verify_profiling_stack_missing_state_file_fails_closed() {
        let missing_path = PathBuf::from("/tmp/nonexistent_profiling_state_12345.json");
        let err = verify_profiling_stack(
            &missing_path,
            "http://127.0.0.1:14567",
            "http://127.0.0.1:18983/solr",
        )
        .unwrap_err();
        assert!(err
            .to_string()
            .contains("profiling-stack state file does not exist"));
    }

    #[test]
    fn test_verify_profiling_stack_directory_state_path_fails_closed() {
        let temp_dir = std::env::temp_dir();
        let err = verify_profiling_stack(
            &temp_dir,
            "http://127.0.0.1:14567",
            "http://127.0.0.1:18983/solr",
        )
        .unwrap_err();
        assert!(err
            .to_string()
            .contains("profiling-stack state path is not a regular file"));
    }

    #[test]
    fn test_verify_profiling_stack_custom_passes_exact_parameters() {
        let temp_dir = std::env::temp_dir().join(format!(
            "test_verify_stack_exact_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&temp_dir).unwrap();
        let state_file = temp_dir.join("profiling_state.json");
        fs::write(&state_file, b"{}").unwrap();

        let executed = Arc::new(AtomicBool::new(false));
        let executed_clone = Arc::clone(&executed);

        let expected_backend = "http://127.0.0.1:14567";
        let expected_solr = "http://127.0.0.1:18983/solr";
        let expected_state = state_file.clone();

        verify_profiling_stack_custom(
            &state_file,
            expected_backend,
            expected_solr,
            |verifier, state, backend, solr| {
                assert!(verifier.is_file());
                assert!(verifier.ends_with("profiling_stack.py"));
                assert_eq!(state, expected_state.as_path());
                assert_eq!(backend, expected_backend);
                assert_eq!(solr, expected_solr);
                executed_clone.store(true, Ordering::SeqCst);
                Ok(())
            },
        )
        .expect("custom verification should succeed");

        assert!(executed.load(Ordering::SeqCst));
        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_verify_profiling_stack_custom_nonzero_exit_fails_closed() {
        let temp_dir = std::env::temp_dir().join(format!(
            "test_verify_stack_nonzero_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&temp_dir).unwrap();
        let state_file = temp_dir.join("profiling_state.json");
        fs::write(&state_file, b"{}").unwrap();

        let err = verify_profiling_stack_custom(
            &state_file,
            "http://127.0.0.1:14567",
            "http://127.0.0.1:18983/solr",
            |_verifier, _state, _backend, _solr| {
                bail!("Profiling stack verification failed (exit code 2): backend mismatch");
            },
        )
        .unwrap_err();

        assert!(err.to_string().contains("exit code 2"));
        assert!(err.to_string().contains("backend mismatch"));
        let _ = fs::remove_dir_all(&temp_dir);
    }
}
