use std::path::{Path, PathBuf};
use std::process::Command;

fn main() {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let ui_dir = manifest_dir.join("ui");

    // Re-run when any frontend input changes.
    println!("cargo:rerun-if-changed=ui/src");
    println!("cargo:rerun-if-changed=ui/package.json");
    println!("cargo:rerun-if-changed=ui/vite.config.ts");
    println!("cargo:rerun-if-changed=ui/index.html");

    println!("cargo:rerun-if-env-changed=SKIP_FRONTEND_BUILD");

    if std::env::var_os("SKIP_FRONTEND_BUILD").is_some() {
        println!("cargo:warning=SKIP_FRONTEND_BUILD set — skipping Vite build");
        return;
    }

    let npm = which("npm").unwrap_or_else(|| {
        panic!("`npm` not found on PATH. Install Node.js or set SKIP_FRONTEND_BUILD=1 if ui/dist/ is already populated.")
    });

    if !ui_dir.join("node_modules").exists() {
        run(&npm, &["install"], &ui_dir, "npm install");
    }

    run(&npm, &["run", "build"], &ui_dir, "npm run build");

    if !ui_dir.join("dist/index.html").exists() {
        panic!("Vite build finished but dist/index.html is missing");
    }
}

fn run(program: &Path, args: &[&str], cwd: &Path, label: &str) {
    let status = Command::new(program)
        .args(args)
        .current_dir(cwd)
        .status()
        .unwrap_or_else(|e| panic!("Failed to spawn {label}: {e}"));
    if !status.success() {
        panic!("{label} failed with status {status}");
    }
}

fn which(bin: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path).find_map(|dir| {
        let candidate = dir.join(bin);
        if candidate.is_file() {
            Some(candidate)
        } else {
            None
        }
    })
}
