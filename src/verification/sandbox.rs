use std::process::Stdio;
use std::time::Duration;

use tokio::io::AsyncWriteExt;
use tokio::process::Command;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VerificationOutcome {
    SyntaxValid { stdout: String },
    SyntaxOrRuntimeError(String),
    Unavailable(String),
}

/// Verifies Python code in a separate isolated Python subprocess (`-I -B`),
/// using a clean temp working directory, stripped environment, and hard timeout.
pub async fn verify_python_isolated(code: &str, timeout_secs: u64) -> VerificationOutcome {
    let temp_dir = std::env::temp_dir().join("knu_diagnosis_sandbox");
    let _ = std::fs::create_dir_all(&temp_dir);

    // Prepend a minimal guard blocking network & process-spawning modules in the child.
    let guarded_script = concat!(
        "import sys, ast\n",
        "_src = sys.stdin.read()\n",
        "ast.parse(_src)\n",
        "for _m in ('socket', 'urllib', 'http', 'subprocess', 'ctypes'):\n",
        "    sys.modules[_m] = None\n",
        "exec(compile(_src, '<knu_diagnosis_verify>', 'exec'), {'__name__': '__main__'})\n",
    );

    let mut cmd = Command::new("python");
    cmd.arg("-I")
        .arg("-B")
        .arg("-c")
        .arg(guarded_script)
        .current_dir(&temp_dir)
        .env_clear()
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);

    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(err) => {
            return VerificationOutcome::Unavailable(format!(
                "Python subprocess unavailable: {err}"
            ));
        }
    };

    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(code.as_bytes()).await;
        let _ = stdin.shutdown().await;
    }

    let timeout = Duration::from_secs(timeout_secs.clamp(1, 10));
    match tokio::time::timeout(timeout, child.wait_with_output()).await {
        Ok(Ok(output)) => {
            let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
            if output.status.success() {
                VerificationOutcome::SyntaxValid { stdout }
            } else {
                VerificationOutcome::SyntaxOrRuntimeError(if stderr.is_empty() {
                    "Python verification failed".to_string()
                } else {
                    stderr
                })
            }
        }
        Ok(Err(err)) => VerificationOutcome::Unavailable(format!("Wait error: {err}")),
        Err(_) => VerificationOutcome::SyntaxOrRuntimeError(format!(
            "Verification timed out after {}s",
            timeout.as_secs()
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn verifies_valid_and_invalid_python_in_subprocess() {
        match verify_python_isolated("print(2 + 3)", 5).await {
            VerificationOutcome::SyntaxValid { stdout } => assert_eq!(stdout, "5"),
            VerificationOutcome::Unavailable(_) => return, // Python not on PATH in CI
            other => panic!("unexpected verification result: {other:?}"),
        }

        match verify_python_isolated("def broken(:", 5).await {
            VerificationOutcome::SyntaxOrRuntimeError(msg) => {
                assert!(msg.contains("SyntaxError"));
            }
            VerificationOutcome::Unavailable(_) => {}
            other => panic!("expected syntax error, got {other:?}"),
        }
    }
}
