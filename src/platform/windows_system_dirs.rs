//! 系统目录跟着系统盘走。系统盘不是 C 盘的机器上，Program Files、ProgramData 和
//! Windows 目录都在别的盘，写死 `C:\` 会让服务校验、救援、修复和升级预检全部失败。
//! 优先向 Windows 查询 Known Folder：这些路径是信任判断的依据，环境变量可以被改。
//! 查询失败时才退到环境变量（改动前的做法），再不行退回旧的固定写法。
use hbb_common::log;
use std::{
    path::{Path, PathBuf},
    sync::OnceLock,
};

/// 旧版代码和打包产物（`bootstrap.json`、救援包）里写死的固定安装目录。
/// 只用来识别旧值并换算成本机的真实目录，不能直接当路径使用。
pub const LEGACY_NORMAL_INSTALL: &str = r"C:\Program Files\RustDesk";
const LEGACY_PROGRAM_FILES: &str = r"C:\Program Files";
const LEGACY_PROGRAM_FILES_X86: &str = r"C:\Program Files (x86)";
const LEGACY_PROGRAM_DATA: &str = r"C:\ProgramData";
const LEGACY_WINDOWS: &str = r"C:\Windows";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SystemDirs {
    pub program_files: PathBuf,
    /// 存量 EXE 安装可能在 32 位目录下，只用于兼容识别。
    pub program_files_x86: PathBuf,
    pub program_data: PathBuf,
    pub windows: PathBuf,
}

#[derive(Debug, Clone, Copy)]
pub enum Folder {
    ProgramFiles,
    ProgramFilesX86,
    ProgramData,
    Windows,
}

impl SystemDirs {
    fn resolve(query: impl Fn(Folder) -> Option<PathBuf>) -> Self {
        let pick = |folder, legacy: &str| {
            query(folder).unwrap_or_else(|| {
                log::warn!("system folder {folder:?} unavailable; using {legacy}");
                PathBuf::from(legacy)
            })
        };
        Self {
            program_files: pick(Folder::ProgramFiles, LEGACY_PROGRAM_FILES),
            program_files_x86: pick(Folder::ProgramFilesX86, LEGACY_PROGRAM_FILES_X86),
            program_data: pick(Folder::ProgramData, LEGACY_PROGRAM_DATA),
            windows: pick(Folder::Windows, LEGACY_WINDOWS),
        }
    }

    pub fn normal_install(&self) -> PathBuf {
        self.program_files.join("RustDesk")
    }

    pub fn rustdesk_data(&self) -> PathBuf {
        self.program_data.join("RustDesk")
    }

    pub fn agent(&self) -> PathBuf {
        self.rustdesk_data().join("ManagementAgent")
    }

    pub fn recovery_bootstrap(&self) -> PathBuf {
        self.rustdesk_data().join("RecoveryBootstrap")
    }

    pub fn client_root(&self) -> PathBuf {
        self.rustdesk_data().join("Client")
    }

    pub fn system32(&self) -> PathBuf {
        self.windows.join("System32")
    }

    pub fn installer_cache(&self) -> PathBuf {
        self.windows.join("Installer")
    }

    /// LocalService 账户的漫游配置目录，服务以它运行时 RustDesk 的配置在这里。
    pub fn local_service_roaming(&self) -> PathBuf {
        self.windows
            .join(r"ServiceProfiles\LocalService\AppData\Roaming")
    }

    /// SYSTEM 账户的漫游配置目录。
    pub fn system_profile_roaming(&self) -> PathBuf {
        self.system32()
            .join(r"config\systemprofile\AppData\Roaming")
    }

    /// 打包产物和旧版本记录的固定安装目录换算成本机真实目录；
    /// 已经是本机固定目录的原样返回，其他任何目录都不是固定布局。
    pub fn fixed_normal_install(&self, value: &Path) -> Option<PathBuf> {
        let value = value.to_string_lossy();
        let value = value.trim_end_matches(['\\', '/']);
        let normal = self.normal_install();
        if value.eq_ignore_ascii_case(LEGACY_NORMAL_INSTALL)
            || value.eq_ignore_ascii_case(&normal.to_string_lossy())
        {
            Some(normal)
        } else {
            None
        }
    }
}

pub fn get() -> &'static SystemDirs {
    static DIRS: OnceLock<SystemDirs> = OnceLock::new();
    DIRS.get_or_init(|| {
        SystemDirs::resolve(|folder| known_folder(folder).or_else(|| environment_folder(folder)))
    })
}

fn environment_folder(folder: Folder) -> Option<PathBuf> {
    let name = match folder {
        Folder::ProgramFiles => "ProgramW6432",
        Folder::ProgramFilesX86 => "ProgramFiles(x86)",
        Folder::ProgramData => "ProgramData",
        Folder::Windows => "SystemRoot",
    };
    std::env::var_os(name)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

fn known_folder(folder: Folder) -> Option<PathBuf> {
    use windows::Win32::{
        System::Com::CoTaskMemFree,
        UI::Shell::{
            FOLDERID_ProgramData, FOLDERID_ProgramFiles, FOLDERID_ProgramFilesX86,
            FOLDERID_Windows, SHGetKnownFolderPath, KF_FLAG_DEFAULT,
        },
    };
    let id = match folder {
        Folder::ProgramFiles => &FOLDERID_ProgramFiles,
        Folder::ProgramFilesX86 => &FOLDERID_ProgramFilesX86,
        Folder::ProgramData => &FOLDERID_ProgramData,
        Folder::Windows => &FOLDERID_Windows,
    };
    let value = unsafe { SHGetKnownFolderPath(id, KF_FLAG_DEFAULT, None) }.ok()?;
    let path = unsafe { value.to_string() };
    unsafe { CoTaskMemFree(Some(value.0.cast())) };
    path.ok().filter(|path| !path.is_empty()).map(PathBuf::from)
}

#[cfg(test)]
mod tests {
    use super::{Folder, SystemDirs, LEGACY_NORMAL_INSTALL};
    use std::path::{Path, PathBuf};

    fn on_drive(drive: &str) -> SystemDirs {
        SystemDirs::resolve(|folder| {
            Some(PathBuf::from(match folder {
                Folder::ProgramFiles => format!(r"{drive}:\Program Files"),
                Folder::ProgramFilesX86 => format!(r"{drive}:\Program Files (x86)"),
                Folder::ProgramData => format!(r"{drive}:\ProgramData"),
                Folder::Windows => format!(r"{drive}:\Windows"),
            }))
        })
    }

    #[test]
    fn every_derived_directory_follows_the_system_drive() {
        let dirs = on_drive("D");
        assert!(dirs.program_files_x86.starts_with(r"D:\"));
        for path in [
            dirs.normal_install(),
            dirs.rustdesk_data(),
            dirs.agent(),
            dirs.recovery_bootstrap(),
            dirs.client_root(),
            dirs.system32(),
            dirs.installer_cache(),
            dirs.local_service_roaming(),
            dirs.system_profile_roaming(),
        ] {
            assert!(path.starts_with(r"D:\"), "{}", path.display());
        }
        assert_eq!(
            dirs.normal_install(),
            Path::new(r"D:\Program Files\RustDesk")
        );
        assert_eq!(
            dirs.agent(),
            Path::new(r"D:\ProgramData\RustDesk\ManagementAgent")
        );
        assert_eq!(
            dirs.local_service_roaming(),
            Path::new(r"D:\Windows\ServiceProfiles\LocalService\AppData\Roaming")
        );
        assert_eq!(
            dirs.system_profile_roaming(),
            Path::new(r"D:\Windows\System32\config\systemprofile\AppData\Roaming")
        );
    }

    #[test]
    fn an_unavailable_folder_falls_back_to_the_previous_fixed_layout() {
        let dirs = SystemDirs::resolve(|folder| match folder {
            Folder::ProgramFiles => Some(PathBuf::from(r"E:\Program Files")),
            _ => None,
        });
        assert_eq!(dirs.program_files, Path::new(r"E:\Program Files"));
        assert_eq!(dirs.program_files_x86, Path::new(r"C:\Program Files (x86)"));
        assert_eq!(dirs.program_data, Path::new(r"C:\ProgramData"));
        assert_eq!(dirs.windows, Path::new(r"C:\Windows"));
    }

    #[test]
    fn the_packaged_fixed_install_marker_maps_to_this_machine() {
        let dirs = on_drive("D");
        let expected = Some(PathBuf::from(r"D:\Program Files\RustDesk"));
        assert_eq!(
            dirs.fixed_normal_install(Path::new(LEGACY_NORMAL_INSTALL)),
            expected
        );
        assert_eq!(
            dirs.fixed_normal_install(Path::new(r"c:\program files\rustdesk\")),
            expected
        );
        assert_eq!(
            dirs.fixed_normal_install(Path::new(r"D:\Program Files\RustDesk")),
            expected
        );
    }

    #[test]
    fn any_other_install_directory_is_not_the_fixed_layout() {
        let dirs = on_drive("D");
        for other in [
            r"E:\Program Files\RustDesk",
            r"D:\RustDesk",
            r"D:\Program Files\RustDesk\sub",
            r"C:\Program Files (x86)\RustDesk",
            "",
        ] {
            assert_eq!(dirs.fixed_normal_install(Path::new(other)), None, "{other}");
        }
    }

    #[test]
    fn this_machine_resolves_existing_system_folders() {
        let dirs = super::get();
        for path in [
            &dirs.program_files,
            &dirs.program_files_x86,
            &dirs.program_data,
            &dirs.windows,
        ] {
            assert!(path.is_absolute() && path.is_dir(), "{}", path.display());
        }
        assert!(dirs.system32().join("msiexec.exe").is_file());
        assert_eq!(
            dirs.program_files,
            PathBuf::from(std::env::var_os("ProgramW6432").expect("ProgramW6432"))
        );
    }
}
