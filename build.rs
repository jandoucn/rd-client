#[cfg(windows)]
fn build_windows() {
    let file = "src/platform/windows.cc";
    let file2 = "src/platform/windows_delete_test_cert.cc";
    cc::Build::new().file(file).file(file2).compile("windows");
    println!("cargo:rustc-link-lib=WtsApi32");
    println!("cargo:rerun-if-changed={}", file);
    println!("cargo:rerun-if-changed={}", file2);
}

#[cfg(windows)]
fn build_agent_version() -> Result<(), Box<dyn std::error::Error>> {
    let metadata: hbb_common::serde_json::Value =
        hbb_common::serde_json::from_str(&std::fs::read_to_string("version.json")?)?;
    let version = metadata["version"].as_str().ok_or("agent version missing")?;
    let parts = version.split('.').map(str::parse::<u16>).collect::<Result<Vec<_>, _>>()?;
    if parts.len() != 3 { return Err("agent semantic version invalid".into()); }
    let build = metadata["build_seq"].as_u64().filter(|value| *value > 0 && *value <= u32::MAX as u64)
        .ok_or("agent build sequence outside Windows version range")?;
    let output = std::path::PathBuf::from(std::env::var_os("OUT_DIR").ok_or("OUT_DIR missing")?);
    let rc = output.join("management-agent-version.rc");
    let resource = output.join("management-agent-version.res");
    // Four WORDs cannot contain semver plus a 32-bit build sequence. The stable
    // numeric order is major/minor/build-high/build-low; strings retain semver.
    let source = format!(r#"#include <winver.h>
1 VERSIONINFO
FILEVERSION {major},{minor},{high},{low}
PRODUCTVERSION {major},{minor},{high},{low}
FILEFLAGSMASK 0x3fL
FILEFLAGS 0
FILEOS VOS_NT_WINDOWS32
FILETYPE VFT_APP
FILESUBTYPE 0
BEGIN
 BLOCK "StringFileInfo"
 BEGIN
  BLOCK "040904e4"
  BEGIN
   VALUE "FileVersion", "{version}+{build}\0"
   VALUE "ProductVersion", "{version}+{build}\0"
   VALUE "FileDescription", "RustDesk Management Agent\0"
   VALUE "ProductName", "RustDesk\0"
   VALUE "OriginalFilename", "rustdesk-management-agent.exe\0"
  END
 END
 BLOCK "VarFileInfo"
 BEGIN
  VALUE "Translation", 0x409, 1252
 END
END
"#, major=parts[0], minor=parts[1], high=build >> 16, low=build & 65535);
    std::fs::write(&rc, source)?;
    let target = std::env::var("TARGET")?;
    let toolset = cc::windows_registry::find_tool(&target, "cl.exe")
        .ok_or("MSVC toolset not found for agent resource compilation")?;
    let tool_path = toolset
        .env()
        .iter()
        .find(|(name, _)| name.to_string_lossy().eq_ignore_ascii_case("PATH"))
        .map(|(_, value)| value.clone())
        .or_else(|| std::env::var_os("PATH"))
        .ok_or("MSVC toolset PATH missing for agent resource compilation")?;
    let resource_compiler = std::env::var_os("WindowsSdkVerBinPath")
        .map(std::path::PathBuf::from)
        .map(|path| path.join(std::env::var("VSCMD_ARG_HOST_ARCH").unwrap_or_else(|_| "x64".into())).join("rc.exe"))
        .filter(|path| path.is_file())
        .or_else(|| {
            std::env::split_paths(&tool_path)
                .map(|path| path.join("rc.exe"))
                .find(|path| path.is_file())
        })
        .ok_or("Windows SDK rc.exe not found for agent resource compilation")?;
    let status = std::process::Command::new(&resource_compiler)
        .envs(toolset.env().iter().cloned())
        .arg("/nologo")
        .arg(format!("/fo{}", resource.display()))
        .arg(&rc)
        .status()
        .map_err(|error| format!("failed to start {}: {error}", resource_compiler.display()))?;
    if !status.success() { return Err(format!("agent resource compiler failed: {status}").into()); }
    println!("cargo:rustc-link-arg-bin=rustdesk-management-agent={}", resource.display());
    println!("cargo:rerun-if-changed=version.json");
    Ok(())
}

#[cfg(target_os = "macos")]
fn build_mac() {
    let file = "src/platform/macos.mm";
    let mut b = cc::Build::new();
    if let Ok(os_version::OsVersion::MacOS(v)) = os_version::detect() {
        let v = v.version;
        if v.contains("10.14") {
            b.flag("-DNO_InputMonitoringAuthStatus=1");
        }
    }
    b.flag("-std=c++17").file(file).compile("macos");
    println!("cargo:rerun-if-changed={}", file);
}

#[cfg(all(windows, feature = "inline"))]
fn build_manifest() {
    use std::io::Write;
    if std::env::var("PROFILE").unwrap() == "release" {
        let mut res = winres::WindowsResource::new();
        res.set_icon("res/icon.ico")
            .set_language(winapi::um::winnt::MAKELANGID(
                winapi::um::winnt::LANG_ENGLISH,
                winapi::um::winnt::SUBLANG_ENGLISH_US,
            ))
            .set_manifest_file("res/manifest.xml");
        match res.compile() {
            Err(e) => {
                write!(std::io::stderr(), "{}", e).unwrap();
                std::process::exit(1);
            }
            Ok(_) => {}
        }
    }
}

// bionic only exports getifaddrs()/freeifaddrs() from API 24, while the jniLibs
// are built against the API 21 sysroot (flutter/ndk_*.sh). webrtc-util calls
// them, so without this the android link fails on undefined symbols.
fn build_android_ifaddrs() {
    let file = "src/platform/android_ifaddrs.c";
    cc::Build::new().file(file).compile("android_ifaddrs");
    println!("cargo:rerun-if-changed={}", file);
}

fn install_android_deps() {
    let target_os = std::env::var("CARGO_CFG_TARGET_OS").unwrap();
    if target_os != "android" {
        return;
    }
    let mut target_arch = std::env::var("CARGO_CFG_TARGET_ARCH").unwrap();
    if target_arch == "x86_64" {
        target_arch = "x64".to_owned();
    } else if target_arch == "x86" {
        target_arch = "x86".to_owned();
    } else if target_arch == "aarch64" {
        target_arch = "arm64".to_owned();
    } else {
        target_arch = "arm".to_owned();
    }
    let target = format!("{}-android", target_arch);
    let vcpkg_root = std::env::var("VCPKG_ROOT").unwrap();
    let mut path: std::path::PathBuf = vcpkg_root.into();
    if let Ok(vcpkg_root) = std::env::var("VCPKG_INSTALLED_ROOT") {
        path = vcpkg_root.into();
    } else {
        path.push("installed");
    }
    path.push(target);
    println!(
        "cargo:rustc-link-search={}",
        path.join("lib").to_str().unwrap()
    );
    println!("cargo:rustc-link-lib=ndk_compat");
    println!("cargo:rustc-link-lib=c++");
    println!("cargo:rustc-link-lib=OpenSLES");
}

fn main() {
    hbb_common::gen_version();
    install_android_deps();
    #[cfg(all(windows, feature = "inline"))]
    build_manifest();
    #[cfg(windows)]
    build_windows();
    #[cfg(windows)]
    if std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc") {
        if let Err(error) = build_agent_version() {
            eprintln!("management agent version resource failed: {error}");
            std::process::exit(1);
        }
    }
    let target_os = std::env::var("CARGO_CFG_TARGET_OS").unwrap();
    if target_os == "macos" {
        #[cfg(target_os = "macos")]
        build_mac();
        println!("cargo:rustc-link-lib=framework=ApplicationServices");
    }
    if target_os == "android" {
        build_android_ifaddrs();
    }
    println!("cargo:rerun-if-changed=build.rs");
}
