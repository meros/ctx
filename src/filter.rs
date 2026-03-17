use anyhow::{Context, Result};
use std::io::Write;
use std::process::{Command, Stdio};

/// Pipes text through `claude -p` with a given prompt for smart filtering.
/// Uses Sonnet with low effort and no tools for speed.
pub fn ask_claude(input: &str, question: &str) -> Result<String> {
    let prompt = format!(
        "You are a context filter. Given the following codebase output, answer this question concisely: {}\n\
         \n\
         Output ONLY the relevant parts — no preamble, no explanation, just the filtered result.\n\
         If nothing is relevant, say \"No relevant results.\"\n\
         \n\
         ---\n\
         {}",
        question, input
    );

    let mut child = Command::new("claude")
        .args([
            "-p",
            "--model", "sonnet",
            "--effort", "low",
            "--tools", "",
            "--disable-slash-commands",
            "--no-session-persistence",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("Failed to launch `claude` CLI. Is it installed?")?;

    {
        let mut stdin = child.stdin.take().expect("stdin was piped");
        stdin.write_all(prompt.as_bytes())?;
        stdin.flush()?;
        // stdin drops here, closing the pipe
    }

    let output = child.wait_with_output()?;

    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("claude CLI failed: {}", stderr)
    }
}

/// If `ask` is Some, filters the output through Claude. Otherwise returns as-is.
pub fn maybe_filter(output: &str, ask: &Option<String>) -> Result<String> {
    match ask {
        Some(question) => ask_claude(output, question),
        None => Ok(output.to_string()),
    }
}
