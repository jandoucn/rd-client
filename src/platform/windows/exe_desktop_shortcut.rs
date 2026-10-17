use std::{
    fs::{File, OpenOptions},
    io::Read,
    os::windows::fs::{MetadataExt, OpenOptionsExt},
    path::{Path, PathBuf},
};
use windows::{
    core::Interface,
    Win32::{Foundation::RPC_E_CHANGED_MODE, System::Com, UI::Shell},
};

type Result<T> = std::result::Result<T, String>;

/// Unlike the legacy install-info fallback, registry errors are never absence.
pub fn installed_target() -> Result<Option<PathBuf>> {
    use winreg::{enums::*, RegKey};
    let machine = RegKey::predef(HKEY_LOCAL_MACHINE);
    for view in [KEY_WOW64_64KEY, KEY_WOW64_32KEY] {
        for key in [
            r"Software\RustDesk\InstallState\RustDesk",
            r"Software\Microsoft\Windows\CurrentVersion\Uninstall\RustDesk",
            r"Software\Microsoft\Windows\CurrentVersion\Uninstall\{54E86BC2-6C85-41F3-A9EB-1A94AC9B1F93}_is1",
        ] {
            let key = match machine.open_subkey_with_flags(key, KEY_READ | view) {
                Ok(key) => key,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
                Err(e) => return Err(format!("installed-state registry unavailable: {e}")),
            };
            let path: String = key
                .get_value("InstallLocation")
                .map_err(|e| format!("installed location unknown: {e}"))?;
            if path.is_empty() {
                return Err("installed location empty".into());
            }
            return Ok(Some(PathBuf::from(path).join("RustDesk.exe")));
        }
    }
    // A missing registration with files still present is not proven first install.
    for id in [
        &Shell::FOLDERID_ProgramFiles,
        &Shell::FOLDERID_ProgramFilesX86,
    ] {
        let target = known_folder(id)?.join("RustDesk/RustDesk.exe");
        match std::fs::symlink_metadata(&target) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(format!("installed executable state unknown: {e}")),
            Ok(_) => return Err("RustDesk files exist without a known installation record".into()),
        }
    }
    Ok(None)
}

/// Keep this value alive until the elevated installation script has completed.
pub struct DesktopPlan {
    _directories: Vec<File>,
    destination: PathBuf,
    create: bool,
    replacement: Vec<u8>,
    original: Option<Vec<u8>>,
    identity: Option<[u32; 3]>,
}

impl DesktopPlan {
    pub fn guard_uninstall(&self, uninstall: &str) -> Result<String> {
        const DELETE: &str = "    if exist \"%PUBLIC%\\Desktop\\RustDesk.lnk\" del /f /q \"%PUBLIC%\\Desktop\\RustDesk.lnk\"";
        let mut found = 0;
        let body = uninstall
            .lines()
            .map(|line| {
                if line == DELETE {
                    found += 1;
                    "rem desktop shortcut retained for handle-bound update"
                } else {
                    line
                }
            })
            .collect::<Vec<_>>()
            .join("\r\n");
        if found != 1 {
            return Err("expected exactly one fixed legacy desktop deletion".into());
        }
        Ok(format!("{}\r\n{body}", self.preflight_commands()?))
    }
    /// Place immediately before the legacy uninstall, in the protected runner.
    pub fn preflight_commands(&self) -> Result<String> {
        let path = self
            .destination
            .to_str()
            .ok_or("non-Unicode desktop")?
            .replace('\'', "''");
        let check = match &self.original {
            Some(bytes) => format!("$b=[IO.File]::ReadAllBytes($p); if([Convert]::ToBase64String($b) -cne '{}'){{throw 'shortcut changed before uninstall'}}", base64(bytes)),
            None => "try {$null=[IO.File]::GetAttributes($p); throw 'shortcut appeared before uninstall'} catch [IO.FileNotFoundException] {}".into(),
        };
        encoded_commands(&format!(
            "$ErrorActionPreference='Stop'; $p='{path}'; {check}; exit 0"
        ))
    }
    pub fn creates_shortcut(&self) -> bool {
        self.create
    }

    /// Runs at the original desktop-copy position, after old uninstall.
    pub fn commands(&self) -> Result<String> {
        if !self.create {
            return Ok(String::new());
        }
        let destination = self.destination.to_str().ok_or("non-Unicode desktop")?;
        let script = if let Some(original) = &self.original {
            format!(
            "$ErrorActionPreference='Stop'; {native}; $p='{path}'; $h=[RDLinkNative]::CreateFileW($p,3221225472,0,[IntPtr]::Zero,3,2097152,[IntPtr]::Zero); if($h.IsInvalid){{$e=[Runtime.InteropServices.Marshal]::GetLastWin32Error();$h.Dispose();if($e -eq 2){{exit 0}};throw ('shortcut open '+$e)}}; try {{$info=New-Object uint32[] 13; if(![RDLinkNative]::GetFileInformationByHandle($h,$info) -or ($info[0] -band 1040) -ne 0 -or $info[10] -ne 1 -or $info[7] -ne {volume} -or $info[11] -ne {high} -or $info[12] -ne {low}){{throw 'shortcut identity changed'}}; $f=[IO.FileStream]::new($h,[IO.FileAccess]::ReadWrite); try {{$old=[Convert]::FromBase64String('{old}'); if($f.Length -ne $old.Length){{throw 'shortcut replaced'}}; $a=New-Object byte[] $old.Length; $n=0; while($n -lt $a.Length){{$r=$f.Read($a,$n,$a.Length-$n);if($r -eq 0){{throw 'short read'}};$n+=$r}}; if([Convert]::ToBase64String($a) -cne [Convert]::ToBase64String($old)){{throw 'shortcut replaced'}}; $b=[Convert]::FromBase64String('{new}'); $f.Position=0; $f.Write($b,0,$b.Length); $f.SetLength($b.Length); $f.Flush($true)}} finally {{$f.Dispose()}}}} finally {{$h.Dispose()}}",
            native=NATIVE, path=destination.replace('\'', "''"), old=base64(original), new=base64(&self.replacement),
            volume=self.identity.ok_or("missing identity")?[0], high=self.identity.ok_or("missing identity")?[1], low=self.identity.ok_or("missing identity")?[2]
        )
        } else {
            format!(
            "$ErrorActionPreference='Stop'; $p='{}'; $b=[Convert]::FromBase64String('{}'); $f=[IO.File]::Open($p,'CreateNew','Write','None'); try {{$f.Write($b,0,$b.Length);$f.Flush($true)}} finally {{$f.Dispose()}}",
            destination.replace('\'', "''"),
            base64(&self.replacement),
        )
        };
        encoded_commands(&script)
    }
}

/// `installed` must come from a successful installed-state lookup BEFORE old
/// uninstall. None means proven first install, never a swallowed lookup error.
/// Existing EXE install writes only PublicDesktop; no personal desktop is read.
pub fn prepare(
    installed: Option<&Path>,
    desktop_option: bool,
    replacement: Vec<u8>,
) -> Result<DesktopPlan> {
    let desktop = known_folder(&Shell::FOLDERID_PublicDesktop)?;
    let public = known_folder(&Shell::FOLDERID_Public)?;
    if desktop != public.join("Desktop") {
        return Err("redirected public desktop differs from legacy uninstall target".into());
    }
    let targets = [
        known_folder(&Shell::FOLDERID_ProgramFiles)?.join("RustDesk/RustDesk.exe"),
        known_folder(&Shell::FOLDERID_ProgramFilesX86)?.join("RustDesk/RustDesk.exe"),
    ];
    if let Some(old) = installed {
        if !targets.iter().any(|target| same_path(old, target)) {
            return Err("installed shortcut target outside fixed RustDesk paths".into());
        }
    }
    let target = link_target(&replacement)?;
    if !targets.iter().any(|allowed| same_path(&target, allowed)) {
        return Err("new shortcut target outside fixed RustDesk paths".into());
    }
    prepare_at(&desktop, installed, desktop_option, replacement)
}

fn prepare_at(
    desktop: &Path,
    installed: Option<&Path>,
    selected: bool,
    replacement: Vec<u8>,
) -> Result<DesktopPlan> {
    let mut directories = Vec::new();
    for ancestor in desktop.ancestors().collect::<Vec<_>>().into_iter().rev() {
        let handle = OpenOptions::new()
            .access_mode(0x80)
            .share_mode(3)
            .custom_flags(0x02000000 | 0x00200000)
            .open(ancestor)
            .map_err(|e| e.to_string())?;
        let metadata = handle.metadata().map_err(|e| e.to_string())?;
        if !metadata.is_dir() || metadata.file_attributes() & 0x400 != 0 {
            return Err("desktop ancestor is not a plain directory".into());
        }
        directories.push(handle);
    }
    let destination = desktop.join("RustDesk.lnk");
    let mut identity = None;
    let original = match OpenOptions::new()
        .read(true)
        .share_mode(1)
        .custom_flags(0x00200000)
        .open(&destination)
    {
        Ok(mut file) => {
            use std::os::windows::io::AsRawHandle;
            #[link(name = "kernel32")]
            extern "system" {
                fn GetFileInformationByHandle(handle: *mut std::ffi::c_void, info: *mut u32)
                    -> i32;
            }
            let mut info = [0u32; 13];
            if unsafe { GetFileInformationByHandle(file.as_raw_handle(), info.as_mut_ptr()) } == 0
                || info[10] != 1
            {
                return Err("shortcut identity unavailable or hardlinked".into());
            }
            identity = Some([info[7], info[11], info[12]]);
            let metadata = file.metadata().map_err(|e| e.to_string())?;
            if !metadata.is_file()
                || metadata.file_attributes() & 0x400 != 0
                || metadata.len() > 1048576
            {
                return Err("shortcut is not a bounded regular file".into());
            }
            let mut bytes = Vec::new();
            file.by_ref()
                .take(1048577)
                .read_to_end(&mut bytes)
                .map_err(|e| e.to_string())?;
            let target = link_target(&bytes)?;
            if !installed.is_some_and(|old| same_path(&target, old)) {
                return Err("existing shortcut does not belong to installed RustDesk".into());
            }
            Some(bytes)
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => return Err(format!("shortcut state unknown: {e}")),
    };
    Ok(DesktopPlan {
        _directories: directories,
        destination,
        create: decision(installed.is_some(), original.is_some(), selected),
        replacement,
        original,
        identity,
    })
}

fn decision(upgrade: bool, present: bool, selected: bool) -> bool {
    if upgrade {
        present
    } else {
        selected
    }
}

fn encoded_commands(script: &str) -> Result<String> {
    let encoded = base64(
        &script
            .encode_utf16()
            .flat_map(u16::to_le_bytes)
            .collect::<Vec<_>>(),
    );
    let command = format!("\"%SystemRoot%\\System32\\WindowsPowerShell\\v1.0\\powershell.exe\" -NoProfile -NonInteractive -EncodedCommand {encoded}");
    if command.len() > 7800 {
        // Windows PowerShell 5.1 interprets BOM-less -File input using ANSI.
        let encoded = base64(&[b"\xEF\xBB\xBF".as_slice(), script.as_bytes()].concat());
        let mut commands = String::new();
        for (index, chunk) in encoded.as_bytes().chunks(120).enumerate() {
            commands.push_str(&format!(
                "{} \"%RUSTDESK_OUTPUT_DIR%\\desktop-write.b64\" echo {}\r\n",
                if index == 0 { ">" } else { ">>" },
                String::from_utf8_lossy(chunk)
            ));
        }
        commands.push_str("certutil -f -decode \"%RUSTDESK_OUTPUT_DIR%\\desktop-write.b64\" \"%RUSTDESK_OUTPUT_DIR%\\desktop-write.ps1\" >nul\r\nif errorlevel 1 exit /b 1\r\n\"%SystemRoot%\\System32\\WindowsPowerShell\\v1.0\\powershell.exe\" -NoProfile -NonInteractive -ExecutionPolicy Bypass -File \"%RUSTDESK_OUTPUT_DIR%\\desktop-write.ps1\"\r\nif errorlevel 1 exit /b 1");
        return Ok(commands);
    }
    Ok(format!("{command}\r\nif errorlevel 1 exit /b 1"))
}

const NATIVE: &str = r#"Add-Type -TypeDefinition 'using System;using System.Runtime.InteropServices;using Microsoft.Win32.SafeHandles;public static class RDLinkNative {[DllImport("kernel32.dll",CharSet=CharSet.Unicode,SetLastError=true)] public static extern SafeFileHandle CreateFileW(string p,uint a,uint s,IntPtr z,uint d,uint f,IntPtr t);[DllImport("kernel32.dll",SetLastError=true)] public static extern bool GetFileInformationByHandle(SafeFileHandle h,[Out] uint[] i);}'"#;

fn same_path(a: &Path, b: &Path) -> bool {
    a.to_string_lossy()
        .replace('/', "\\")
        .eq_ignore_ascii_case(&b.to_string_lossy().replace('/', "\\"))
}

struct ComGuard(bool);
impl Drop for ComGuard {
    fn drop(&mut self) {
        if self.0 {
            unsafe { Com::CoUninitialize() };
        }
    }
}
fn com() -> Result<ComGuard> {
    let hr = unsafe { Com::CoInitializeEx(None, Com::COINIT_APARTMENTTHREADED) };
    if hr.is_err() && hr != RPC_E_CHANGED_MODE {
        return Err(format!("COM initialization: {hr:?}"));
    }
    Ok(ComGuard(hr.is_ok()))
}
fn link_target(bytes: &[u8]) -> Result<PathBuf> {
    let _com = com()?;
    let link: Shell::IShellLinkW =
        unsafe { Com::CoCreateInstance(&Shell::ShellLink, None, Com::CLSCTX_INPROC_SERVER) }
            .map_err(|e| e.to_string())?;
    let stream =
        unsafe { Shell::SHCreateMemStream(Some(bytes)) }.ok_or("shortcut stream unavailable")?;
    let persist: Com::IPersistStream = link.cast().map_err(|e| e.to_string())?;
    unsafe { persist.Load(&stream) }.map_err(|e| e.to_string())?;
    let mut buffer = vec![0u16; 32768];
    unsafe {
        link.GetPath(
            &mut buffer,
            std::ptr::null_mut(),
            Shell::SLGP_RAWPATH.0 as u32,
        )
    }
    .map_err(|e| e.to_string())?;
    let end = buffer
        .iter()
        .position(|v| *v == 0)
        .ok_or("unterminated shortcut target")?;
    if end == 0 {
        return Err("empty shortcut target".into());
    }
    Ok(PathBuf::from(
        String::from_utf16(&buffer[..end]).map_err(|e| e.to_string())?,
    ))
}
fn known_folder(id: &windows::core::GUID) -> Result<PathBuf> {
    let value = unsafe { Shell::SHGetKnownFolderPath(id, Shell::KF_FLAG_DONT_VERIFY, None) }
        .map_err(|e| e.to_string())?;
    let text = unsafe { value.to_string() };
    unsafe { Com::CoTaskMemFree(Some(value.0.cast())) };
    Ok(PathBuf::from(text.map_err(|e| e.to_string())?))
}
fn base64(bytes: &[u8]) -> String {
    const TABLE: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for chunk in bytes.chunks(3) {
        let n = (u32::from(chunk[0]) << 16)
            | (u32::from(*chunk.get(1).unwrap_or(&0)) << 8)
            | u32::from(*chunk.get(2).unwrap_or(&0));
        out.push(TABLE[(n >> 18) as usize] as char);
        out.push(TABLE[((n >> 12) & 63) as usize] as char);
        out.push(if chunk.len() > 1 {
            TABLE[((n >> 6) & 63) as usize] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            TABLE[(n & 63) as usize] as char
        } else {
            '='
        });
    }
    out
}

#[cfg(test)]
#[path = "../../../tests/exe_desktop_adapter/cases.rs"]
mod tests;
