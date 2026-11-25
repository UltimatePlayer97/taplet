fn main() {
    // Embed the short git commit for `taplet doctor`. Silent when git or .git is absent.
    let commit = std::process::Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .filter(|c| !c.is_empty());
    if let Some(commit) = commit {
        println!("cargo:rustc-env=GIT_COMMIT={commit}");
    }
    println!("cargo:rerun-if-changed=.git/HEAD");
}
