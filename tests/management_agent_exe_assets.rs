#![cfg(windows)]
#[path = "../src/platform/windows_system_dirs.rs"]
#[allow(dead_code)]
mod system_dirs;
// 代理源码里有 `crate::management_agent::system_dirs` 的写法，测试入口补同名路径。
#[allow(unused_imports)]
mod management_agent {
    pub(crate) use super::system_dirs;
}
#[path = "../src/management_agent/exe_assets.rs"]
mod exe_assets;
#[path = "../src/management_agent/package_gate.rs"]
mod package_gate;
#[path = "../src/management_agent/workspace.rs"]
mod workspace;

use exe_assets::AssetError;
use hbb_common::{
    base64::{engine::general_purpose::STANDARD, Engine as _},
    sodiumoxide::crypto::sign,
    UpdateManifest, UpdateTarget,
};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

fn lab() -> PathBuf {
    base::lab_guard::require_lab_os("win10");
    PathBuf::from(r"C:\RustDeskAgentLab\exe-v215")
}
struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let path = lab().join(format!("case-{}", uuid::Uuid::new_v4().simple()));
        workspace::create(&path).unwrap();
        std::fs::write(path.join("old-cache"), b"old usable bytes").unwrap();
        Self(path)
    }
    fn preserved(&self) {
        assert_eq!(
            std::fs::read(self.0.join("old-cache")).unwrap(),
            b"old usable bytes"
        );
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.preserved();
        std::fs::remove_dir_all(&self.0).unwrap();
    }
}

fn signed(package: &Path, edition: &str) -> (UpdateManifest, sign::PublicKey) {
    hbb_common::sodiumoxide::init().unwrap();
    let (public, secret) = sign::gen_keypair();
    let bytes = std::fs::read(package).unwrap();
    let mut manifest = UpdateManifest {
        schema: 2,
        product: "rustdesk-yan".into(),
        edition: "multi".into(),
        channel: "stable".into(),
        ..Default::default()
    };
    let key = format!("windows-x86_64-exe-{edition}");
    let mut target = UpdateTarget {
        version: "1.5.3".into(),
        build_seq: 2026100504,
        source_commit: "exe-v215-fixture".into(),
        primary: "https://download.yan.life/rustdesk/stable/1.5.3/fixture.exe".into(),
        size: bytes.len() as u64,
        sha256: hex::encode(Sha256::digest(bytes)),
        signature_key_id: "fixture-key".into(),
        ..Default::default()
    };
    target.signature = STANDARD.encode(sign::sign_detached(
        &base::update::update_signature_payload(&manifest, &key, &target).unwrap(),
        &secret,
    ));
    manifest.targets.insert(key, target);
    (manifest, public)
}

#[test]
fn exact_exe_identity_accepts_each_edition_and_rejects_cross_kind_or_unknown() {
    lab();
    for edition in ["standard", "sos"] {
        assert_eq!(
            exe_assets::check_identity("exe", edition, &format!("windows-x86_64-exe-{edition}")),
            Ok(())
        );
        for kind in ["msi", "", "unknown"] {
            assert_eq!(
                exe_assets::check_identity(kind, edition, &format!("windows-x86_64-exe-{edition}")),
                Err(AssetError::Identity)
            );
        }
    }
    for key in [
        "windows-x86_64-exe-sos",
        "windows-x86_64-msi-standard",
        "windows-aarch64-exe-standard",
    ] {
        assert_eq!(
            exe_assets::check_identity("exe", "standard", key),
            Err(AssetError::Identity)
        );
    }
}

#[test]
fn signed_real_packer_extracts_complete_inventory_and_keeps_old_data() {
    let f = Fixture::new();
    let package = lab().join("fixture.exe");
    let before = std::fs::read(&package).unwrap();
    for edition in ["standard", "sos"] {
        let (manifest, public) = signed(&package, edition);
        let key = format!("windows-x86_64-exe-{edition}");
        let sealed = exe_assets::prepare_fixture(
            &f.0,
            &package,
            &manifest,
            &key,
            "exe",
            edition,
            2026100502,
            public.as_ref(),
        )
        .unwrap();
        assert!(sealed.root.starts_with(&f.0));
        assert!(sealed.root.join("sealed.json").is_file());
        for (path, size, hash) in &sealed.files {
            let bytes = std::fs::read(sealed.root.join("payload").join(path)).unwrap();
            assert_eq!(bytes.len() as u64, *size);
            assert_eq!(hex::encode(Sha256::digest(bytes)), *hash);
        }
        for file in [
            "rustdesk.exe",
            "librustdesk.dll",
            "rustdesk-management-agent.exe",
            "flutter_windows.dll",
            "window_manager_plugin.dll",
            "data/flutter_assets/AssetManifest.json",
        ] {
            assert!(sealed.root.join("payload").join(file).is_file(), "{file}");
        }
    }
    assert_eq!(std::fs::read(package).unwrap(), before);
    f.preserved();
}

#[test]
fn rejected_signature_identity_and_size_never_create_staging() {
    let f = Fixture::new();
    let package = lab().join("fixture.exe");
    let (manifest, public) = signed(&package, "standard");
    let key = "windows-x86_64-exe-standard";
    assert_eq!(
        exe_assets::prepare_fixture(&f.0, &package, &manifest, key, "exe", "standard", 1, &[0; 32])
            .unwrap_err(),
        AssetError::Signature
    );
    assert_eq!(
        exe_assets::prepare_fixture(
            &f.0,
            &package,
            &manifest,
            key,
            "msi",
            "standard",
            1,
            public.as_ref()
        )
        .unwrap_err(),
        AssetError::Identity
    );
    assert_eq!(std::fs::read_dir(&f.0).unwrap().count(), 1);
    f.preserved();
}

#[test]
fn malformed_real_packer_inventory_is_rejected_without_extracting_payload() {
    for variant in ["missing", "escape", "corrupt"] {
        let f = Fixture::new();
        let package = lab().join(format!("fixture-{variant}.exe"));
        let (manifest, public) = signed(&package, "standard");
        let before = std::fs::read(&package).unwrap();
        assert_eq!(
            exe_assets::prepare_fixture(
                &f.0,
                &package,
                &manifest,
                "windows-x86_64-exe-standard",
                "exe",
                "standard",
                1,
                public.as_ref()
            )
            .unwrap_err(),
            AssetError::Extraction
        );
        for entry in std::fs::read_dir(&f.0).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                assert!(!path.join("payload").exists());
                assert!(!path.join("sealed.json").exists());
            }
        }
        assert_eq!(std::fs::read(package).unwrap(), before);
        assert!(!f.0.join("escape-sentinel").exists());
        f.preserved();
    }
}

#[test]
fn changed_package_reparse_and_writable_workspace_preserve_old_bytes() {
    let f = Fixture::new();
    let source = lab().join("fixture.exe");
    let package = f.0.join("source.exe");
    std::fs::copy(&source, &package).unwrap();
    let (manifest, public) = signed(&package, "standard");
    let key = "windows-x86_64-exe-standard";
    std::fs::write(&package, b"corrupt source").unwrap();
    assert_eq!(
        exe_assets::prepare_fixture(
            &f.0,
            &package,
            &manifest,
            key,
            "exe",
            "standard",
            1,
            public.as_ref()
        )
        .unwrap_err(),
        AssetError::Package
    );
    assert_eq!(std::fs::read(&package).unwrap(), b"corrupt source");
    std::fs::remove_file(&package).unwrap();
    std::os::windows::fs::symlink_file(&source, &package).unwrap();
    assert_eq!(
        exe_assets::prepare_fixture(
            &f.0,
            &package,
            &manifest,
            key,
            "exe",
            "standard",
            1,
            public.as_ref()
        )
        .unwrap_err(),
        AssetError::Package
    );
    std::fs::remove_file(package).unwrap();
    assert!(
        std::process::Command::new(r"C:\Windows\System32\icacls.exe")
            .arg(&f.0)
            .args(["/grant", "*S-1-5-32-545:(W)"])
            .status()
            .unwrap()
            .success()
    );
    assert_eq!(
        exe_assets::prepare_fixture(
            &f.0,
            &source,
            &manifest,
            key,
            "exe",
            "standard",
            1,
            public.as_ref()
        )
        .unwrap_err(),
        AssetError::Workspace
    );
    f.preserved();
}

#[test]
fn strict_packer_refuses_arbitrary_or_occupied_output_without_touching_old_data() {
    let f = Fixture::new();
    let package = f.0.join("package.exe");
    std::fs::copy(lab().join("fixture.exe"), &package).unwrap();
    let arbitrary = f.0.join("not-payload");
    let status = std::process::Command::new(&package)
        .args([
            "--management-agent-strict-assets",
            "--management-agent-extract-path",
        ])
        .arg(&arbitrary)
        .status()
        .unwrap();
    assert!(!status.success());
    assert!(!arbitrary.exists());
    let payload = f.0.join("payload");
    workspace::create(&payload).unwrap();
    std::fs::write(payload.join("keep"), b"keep old output").unwrap();
    assert!(!std::process::Command::new(&package)
        .args([
            "--management-agent-strict-assets",
            "--management-agent-extract-path"
        ])
        .arg(&payload)
        .status()
        .unwrap()
        .success());
    assert_eq!(
        std::fs::read(payload.join("keep")).unwrap(),
        b"keep old output"
    );
    f.preserved();
}

#[test]
fn inventory_size_count_alias_and_completeness_bounds_are_enforced() {
    lab();
    let line = |name: &str, size: u64| format!("{name}\t{size}\t{}\n", "a".repeat(64));
    for path in [
        "../escape",
        "/escape",
        "a:stream",
        "a\\b",
        "NUL.exe",
        "a./b",
    ] {
        let text = format!("rustdesk-strict-assets-v1\n{}end\n", line(path, 1));
        assert_eq!(
            exe_assets::validate_inventory_fixture(text.as_bytes()),
            Err(AssetError::Inventory)
        );
    }
    for size in [0, 512 * 1024 * 1024 + 1, u64::MAX] {
        let text = format!(
            "rustdesk-strict-assets-v1\n{}end\n",
            line("rustdesk.exe", size)
        );
        assert_eq!(
            exe_assets::validate_inventory_fixture(text.as_bytes()),
            Err(AssetError::Limit)
        );
    }
    let mut text = String::from("rustdesk-strict-assets-v1\n");
    for i in 0..17 {
        text.push_str(&line(&format!("{i}.dll"), 512 * 1024 * 1024));
    }
    text.push_str("end\n");
    assert_eq!(
        exe_assets::validate_inventory_fixture(text.as_bytes()),
        Err(AssetError::Limit)
    );
    let mut text = String::from("rustdesk-strict-assets-v1\n");
    for i in 0..20001 {
        text.push_str(&line(&format!("{i}.dll"), 1));
    }
    text.push_str("end\n");
    assert_eq!(
        exe_assets::validate_inventory_fixture(text.as_bytes()),
        Err(AssetError::Limit)
    );
    assert_eq!(
        exe_assets::validate_inventory_fixture(b"rustdesk-strict-assets-v1\nend\n"),
        Err(AssetError::Inventory)
    );
}

#[test]
fn sealed_inventory_detects_tampered_missing_extra_and_reparse_output() {
    let f = Fixture::new();
    let package = lab().join("fixture.exe");
    let (manifest, public) = signed(&package, "standard");
    let sealed = exe_assets::prepare_fixture(
        &f.0,
        &package,
        &manifest,
        "windows-x86_64-exe-standard",
        "exe",
        "standard",
        1,
        public.as_ref(),
    )
    .unwrap();
    let root = sealed.root.join("payload");
    let dll = root.join("librustdesk.dll");
    let bytes = std::fs::read(&dll).unwrap();
    let seal = std::fs::read(sealed.root.join("sealed.json")).unwrap();
    std::fs::write(&dll, b"tampered").unwrap();
    assert_eq!(
        exe_assets::verify_output_fixture(&sealed),
        Err(AssetError::Integrity)
    );
    std::fs::remove_file(&dll).unwrap();
    assert_eq!(
        exe_assets::verify_output_fixture(&sealed),
        Err(AssetError::Integrity)
    );
    std::os::windows::fs::symlink_file(f.0.join("old-cache"), &dll).unwrap();
    assert_eq!(
        exe_assets::verify_output_fixture(&sealed),
        Err(AssetError::Integrity)
    );
    std::fs::remove_file(&dll).unwrap();
    std::fs::write(&dll, &bytes).unwrap();
    std::fs::create_dir(root.join("extra-directory")).unwrap();
    assert_eq!(
        exe_assets::verify_output_fixture(&sealed),
        Err(AssetError::Integrity)
    );
    std::fs::remove_dir(root.join("extra-directory")).unwrap();
    assert!(exe_assets::verify_output_fixture(&sealed).is_ok());
    assert_eq!(
        std::fs::read(sealed.root.join("sealed.json")).unwrap(),
        seal
    );
    f.preserved();
}

fn resource_copy(root: &Path, name: &str, blob: Option<&[u8]>, zero: bool) -> PathBuf {
    use std::os::windows::ffi::OsStrExt;
    #[link(name = "kernel32")]
    extern "system" {
        fn BeginUpdateResourceW(path: *const u16, delete: i32) -> *mut std::ffi::c_void;
        fn UpdateResourceW(
            handle: *mut std::ffi::c_void,
            kind: *const u16,
            name: *const u16,
            language: u16,
            data: *const u8,
            size: u32,
        ) -> i32;
        fn EndUpdateResourceW(handle: *mut std::ffi::c_void, discard: i32) -> i32;
    }
    let path = root.join(format!("{name}.exe"));
    std::fs::copy(lab().join("fixture.exe"), &path).unwrap();
    if let Some(blob) = blob {
        let wide = path
            .as_os_str()
            .encode_wide()
            .chain(Some(0))
            .collect::<Vec<_>>();
        let name = "RDPKG\0".encode_utf16().collect::<Vec<_>>();
        unsafe {
            let handle = BeginUpdateResourceW(wide.as_ptr(), 0);
            assert!(!handle.is_null());
            let updated = UpdateResourceW(
                handle,
                10usize as *const u16,
                name.as_ptr(),
                0,
                blob.as_ptr(),
                blob.len() as u32,
            );
            if updated == 0 {
                let error = std::io::Error::last_os_error();
                EndUpdateResourceW(handle, 1);
                panic!("resource fixture update: {error}");
            }
            assert_ne!(EndUpdateResourceW(handle, 0), 0);
        }
    }
    if zero {
        let mut bytes = std::fs::read(&path).unwrap();
        let u16at =
            |b: &[u8], p: usize| u16::from_le_bytes(b[p..p + 2].try_into().unwrap()) as usize;
        let u32at =
            |b: &[u8], p: usize| u32::from_le_bytes(b[p..p + 4].try_into().unwrap()) as usize;
        let pe = u32at(&bytes, 60);
        let opt = pe + 24;
        let sections = opt + u16at(&bytes, pe + 20);
        let rva = u32at(&bytes, opt + 128);
        let base = (0..u16at(&bytes, pe + 6))
            .find_map(|i| {
                let s = sections + 40 * i;
                let va = u32at(&bytes, s + 12);
                let size = u32at(&bytes, s + 16);
                (rva >= va && rva < va + size).then(|| u32at(&bytes, s + 20) + rva - va)
            })
            .unwrap();
        let mut dir = base;
        for level in 0..3 {
            dir = (0..u16at(&bytes, dir + 12) + u16at(&bytes, dir + 14))
                .find_map(|i| {
                    let e = dir + 16 + 8 * i;
                    let name = u32at(&bytes, e);
                    let value = u32at(&bytes, e + 4);
                    let matches = if level == 0 {
                        name == 10
                    } else if level == 1 && name & 0x80000000 != 0 {
                        let p = base + (name & 0x7fffffff);
                        let n = u16at(&bytes, p);
                        String::from_utf16(
                            &bytes[p + 2..p + 2 + n * 2]
                                .chunks_exact(2)
                                .map(|v| u16::from_le_bytes([v[0], v[1]]))
                                .collect::<Vec<_>>(),
                        )
                        .unwrap()
                            == "RDPKG"
                    } else {
                        level == 2
                    };
                    matches.then_some(base + (value & 0x7fffffff))
                })
                .unwrap();
        }
        bytes[dir + 4..dir + 8].copy_from_slice(&0u32.to_le_bytes());
        std::fs::write(&path, bytes).unwrap();
    }
    path
}

fn resource_size(path: &Path) -> Option<u32> {
    use std::os::windows::ffi::OsStrExt;
    #[link(name = "kernel32")]
    extern "system" {
        fn LoadLibraryExW(
            path: *const u16,
            file: *mut std::ffi::c_void,
            flags: u32,
        ) -> *mut std::ffi::c_void;
        fn FindResourceW(
            module: *mut std::ffi::c_void,
            name: *const u16,
            kind: *const u16,
        ) -> *mut std::ffi::c_void;
        fn SizeofResource(module: *mut std::ffi::c_void, resource: *mut std::ffi::c_void) -> u32;
        fn FreeLibrary(module: *mut std::ffi::c_void) -> i32;
    }
    let path = path
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect::<Vec<_>>();
    let name = "RDPKG\0".encode_utf16().collect::<Vec<_>>();
    unsafe {
        let module = LoadLibraryExW(path.as_ptr(), std::ptr::null_mut(), 2);
        assert!(!module.is_null());
        let resource = FindResourceW(module, name.as_ptr(), 10usize as *const u16);
        let size = if resource.is_null() {
            None
        } else {
            Some(SizeofResource(module, resource))
        };
        assert_ne!(FreeLibrary(module), 0);
        size
    }
}

#[test]
fn real_pe_rdpk_absent_zero_override_and_traversal_have_distinct_results() {
    let f = Fixture::new();
    let valid=STANDARD.decode("cnVzdGRlc2sAAAAmZGF0YS9mbHV0dGVyX2Fzc2V0cy9Bc3NldE1hbmlmZXN0Lmpzb24AAAAeGxkA+I1Utb9sISvLsjS5NkQRSdKRK/RiE6Xs4gN2YTczMWIyMTQwYWVkZWNiYTI0Yzk0NjY0NjQyNDhlNzdydXN0ZGVza3J1c3RkZXNrLmV4ZQ==").unwrap();
    let escape=STANDARD.decode("cnVzdGRlc2sAAAASLi4vZXNjYXBlLXNlbnRpbmVsAAAAChsFAPglAIKQMAVlMGViYzNjNDA5MDcwZDA3ZjFkZjBmMmY0MTMyNTA5ZXJ1c3RkZXNrcnVzdGRlc2suZXhl").unwrap();
    let mut zero_accepted = false;
    for (name, blob, zero, accepted) in [
        ("absent", None, false, true),
        ("zero", Some(&[1u8][..]), true, false),
        ("override", Some(valid.as_slice()), false, true),
        ("traversal", Some(escape.as_slice()), false, false),
    ] {
        let package = resource_copy(&f.0, name, blob, zero);
        assert_eq!(
            resource_size(&package),
            blob.map(|b| if zero { 0 } else { b.len() as u32 }),
            "resource fixture {name}"
        );
        let output = std::process::Command::new(&package)
            .args([
                "--management-agent-strict-assets",
                "--management-agent-inventory",
            ])
            .output()
            .unwrap();
        let text = String::from_utf8(output.stdout).unwrap();
        println!(
            "RDPKG_PROBE={name};exit={:?};sha256={};stderr={}",
            output.status.code(),
            hex::encode(Sha256::digest(std::fs::read(&package).unwrap())),
            String::from_utf8_lossy(&output.stderr)
        );
        if name == "zero" {
            zero_accepted = output.status.success();
        } else {
            assert_eq!(output.status.success(), accepted, "{name}");
        }
        if name == "override" {
            assert!(
                text.contains("6c8bbd8106b32bf055c46d8b5213d4a5bd18344eeba92fd4339b650246683c53")
            );
        }
        if !accepted && !output.status.success() {
            assert!(String::from_utf8_lossy(&output.stderr).contains(if zero {
                "strict:resource"
            } else {
                "strict:path"
            }));
        }
        if accepted {
            let (manifest, public) = signed(&package, "standard");
            let sealed = exe_assets::prepare_fixture(
                &f.0,
                &package,
                &manifest,
                "windows-x86_64-exe-standard",
                "exe",
                "standard",
                1,
                public.as_ref(),
            )
            .unwrap();
            if name == "override" {
                assert_eq!(
                    std::fs::read(
                        sealed
                            .root
                            .join("payload/data/flutter_assets/assetmanifest.json")
                    )
                    .unwrap(),
                    b"resource-override-verified"
                );
            }
        } else if !output.status.success() {
            let (manifest, public) = signed(&package, "standard");
            assert_eq!(
                exe_assets::prepare_fixture(
                    &f.0,
                    &package,
                    &manifest,
                    "windows-x86_64-exe-standard",
                    "exe",
                    "standard",
                    1,
                    public.as_ref()
                )
                .unwrap_err(),
                AssetError::Extraction
            );
            assert!(!f.0.join("payload").exists());
        }
    }
    f.preserved();
    assert!(
        !zero_accepted,
        "existing zero-length RDPKG must not become absent"
    );
}
