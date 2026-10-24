use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

/// Write `contents` to `path` as an executable. A child process writes it,
/// not this one: a file this process held open for writing would be
/// inherited by whatever another test spawns meanwhile, and couldn't be run
/// until that let go of it ("Text file busy"). For the same reason, copy
/// executables with `cp` in a child process.
pub fn write_executable(path: &Path, contents: &str) {
    let mut child = Command::new("sh")
        .args(["-c", "cat > \"$1\" && chmod 755 \"$1\"", "sh"])
        .arg(path)
        .stdin(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(contents.as_bytes())
        .unwrap();
    let status = child.wait().unwrap();
    assert!(status.success(), "could not write {}", path.display());
}
