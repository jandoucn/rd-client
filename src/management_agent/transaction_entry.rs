use std::path::{Path, PathBuf};

pub fn resolve(root: &Path, executable: &Path, arguments: &[String]) -> Result<PathBuf, String> {
    if !root.is_absolute() {
        return Err("transaction root must be absolute".into());
    }
    if arguments.len() != 2 || arguments[0] != "--transaction-id"
        || arguments[1].len() != 32
        || !arguments[1].bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)) {
        return Err("invalid transaction helper arguments".into());
    }
    let directory = root.join(&arguments[1]);
    if executable != directory.join("runtime").join("helper.exe") {
        return Err("transaction helper executable outside bound runtime".into());
    }
    Ok(directory)
}
