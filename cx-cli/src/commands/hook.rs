use anyhow::{Context, Result};
use std::path::Path;

/// Install or remove the cx post-commit git hook.
pub fn run(root: &Path, install: bool, remove: bool) -> Result<()> {
    let hooks_dir = root.join(".git").join("hooks");
    let hook_path = hooks_dir.join("post-commit");

    if !hooks_dir.exists() {
        anyhow::bail!("Not a git repository (no .git/hooks/)");
    }

    if remove {
        if hook_path.exists() {
            let content = std::fs::read_to_string(&hook_path).unwrap_or_default();
            if content.contains("cx build") {
                std::fs::remove_file(&hook_path)
                    .context("failed to remove hook")?;
                eprintln!("Removed post-commit hook");
            } else {
                eprintln!("post-commit hook exists but wasn't installed by cx, skipping");
            }
        } else {
            eprintln!("No post-commit hook found");
        }
        return Ok(());
    }

    if install {
        if hook_path.exists() {
            let content = std::fs::read_to_string(&hook_path).unwrap_or_default();
            if content.contains("cx build") {
                eprintln!("cx post-commit hook already installed");
                return Ok(());
            }
            eprintln!("Warning: post-commit hook already exists, appending cx build");
            let mut content = content;
            content.push_str("\n# cx: update graph after commit\ncx build 2>/dev/null &\n");
            std::fs::write(&hook_path, content)
                .context("failed to update hook")?;
        } else {
            let content = "#!/bin/sh\n# cx: update graph after commit (cached LLM results make this fast)\ncx build 2>/dev/null &\n";
            std::fs::write(&hook_path, content)
                .context("failed to write hook")?;
        }

        // Make executable
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = std::fs::metadata(&hook_path)?.permissions();
            perms.set_mode(0o755);
            std::fs::set_permissions(&hook_path, perms)?;
        }

        eprintln!("Installed post-commit hook: .git/hooks/post-commit");
        eprintln!("cx build will run in the background after each commit.");
        eprintln!("LLM results are cached — subsequent builds are fast (~2s).");
        return Ok(());
    }

    // Default: show status
    if hook_path.exists() {
        let content = std::fs::read_to_string(&hook_path).unwrap_or_default();
        if content.contains("cx build") {
            println!("cx post-commit hook: installed");
        } else {
            println!("post-commit hook exists but not managed by cx");
        }
    } else {
        println!("cx post-commit hook: not installed");
        println!("Run `cx hook --install` to auto-update the graph after each commit.");
    }

    Ok(())
}
