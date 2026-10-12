use std::{io::Write, path::Path};

#[cfg(windows)]
pub fn publish(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    #[link(name = "kernel32")]
    extern "system" {
        fn MoveFileExW(source: *const u16, destination: *const u16, flags: u32) -> i32;
        fn SetFileAttributesW(path: *const u16, attributes: u32) -> i32;
    }
    let temporary = path.with_extension(format!("tmp-{}", std::process::id()));
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)?;
    let result = (|| {
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        let source: Vec<u16> = temporary.as_os_str().encode_wide().chain(Some(0)).collect();
        let destination: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
        if unsafe { SetFileAttributesW(source.as_ptr(), 2 | 4) } == 0 {
            return Err(std::io::Error::last_os_error());
        }
        if unsafe { MoveFileExW(source.as_ptr(), destination.as_ptr(), 1 | 8) } == 0 {
            return Err(std::io::Error::last_os_error());
        }
        Ok(())
    })();
    if let Err(error) = result {
        if let Err(cleanup) = std::fs::remove_file(&temporary) {
            return Err(std::io::Error::new(
                error.kind(),
                format!("{error}; state staging cleanup failed: {cleanup}"),
            ));
        }
        return Err(error);
    }
    Ok(())
}
