#![cfg(windows)]

#[path = "../src/management_agent/first_install_approval_delivery.rs"]
mod first_install_approval_delivery;
#[path = "../src/platform/windows_agent_guard.rs"]
mod install_guard;
#[path = "../src/management_agent/msi_metadata.rs"]
mod msi_metadata;
#[path = "../src/management_agent/package_gate.rs"]
mod package_gate;
#[path = "../src/management_agent/workspace.rs"]
mod workspace;

use first_install_approval_delivery::{deliver_at, DeliveryError, Request, TestLayout};
use std::path::{Path, PathBuf};

const BUILD: u64 = 2026101501;
const INSTALLATION: &str = "cccccccccccccccccccccccccccccccc";
const KEY: &str = "windows-x86_64-msi-sos";

struct Fixture {
    root: PathBuf,
    manifest: Vec<u8>,
    package: PathBuf,
    product_code: String,
}

impl Fixture {
    fn new() -> Self {
        base::lab_guard::require_lab_os("win10");
        assert_eq!(
            std::env::var("RUSTDESK_T01_PRODUCER").as_deref(),
            Ok("red-formal-sos-1501"),
            "INPUT BLOCKED: explicit owner selection required"
        );
        let input = Path::new(r"C:\RustDeskAgentLab\t01-delivery-input");
        let reply: serde_json::Value =
            serde_json::from_slice(&std::fs::read(input.join("resolved.json")).unwrap()).unwrap();
        let manifest =
            package_gate::bind_resolved(&reply, reply["package_id"].as_str().unwrap(), KEY, BUILD)
                .expect("INPUT BLOCKED: formal resolved binding");
        let target = package_gate::verify(&manifest, KEY, BUILD)
            .expect("INPUT BLOCKED: original production signature");
        let package = input.join(package_gate::cache_name(&target.primary, "msi").unwrap());
        let _locked = package_gate::lock_verified_file(&package, target)
            .expect("INPUT BLOCKED: original MSI bytes");
        let product_code = msi_metadata::inspect(&package)
            .expect("INPUT BLOCKED: real MSI metadata")
            .product_code;
        let root = Path::new(r"C:\RustDeskAgentLab").join(format!(
            "t01-producer-red-{}",
            uuid::Uuid::new_v4().simple()
        ));
        workspace::create(&root).unwrap();
        workspace::create(&root.join("state")).unwrap();
        workspace::create(&root.join("release-cache")).unwrap();
        std::fs::write(root.join("state/service-recovery-enabled"), b"enabled").unwrap();
        std::fs::write(
            root.join("state/agent.json"),
            serde_json::to_vec(&serde_json::json!({
                "agent_installation_id": INSTALLATION, "target_key": KEY,
                "management_agent_enabled": true, "disabled": false
            }))
            .unwrap(),
        )
        .unwrap();
        hbb_common::sodiumoxide::init().unwrap();
        let (public, secret) = hbb_common::sodiumoxide::crypto::sign::gen_keypair();
        std::fs::write(
            root.join("identity.toml"),
            format!(
                "id = 'fixture-identity'\nkey_pair = [{:?}, {:?}]\n",
                secret.0.to_vec(),
                public.0.to_vec()
            ),
        )
        .unwrap();
        std::fs::write(root.join("config.toml"), b"[options]\nstop-service = 'N'\n").unwrap();
        println!("T01 producer fixture: {}", root.display());
        Self {
            root,
            manifest: serde_json::to_vec(&reply["manifest"]).unwrap(),
            package,
            product_code,
        }
    }

    fn install_state_acl(&self) {
        use std::os::windows::ffi::OsStrExt;
        use windows::{
            core::PCWSTR,
            Win32::{
                Foundation::{LocalFree, HLOCAL},
                Security::{
                    Authorization::{
                        ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1,
                    },
                    SetFileSecurityW, DACL_SECURITY_INFORMATION, OWNER_SECURITY_INFORMATION,
                    PSECURITY_DESCRIPTOR, UNPROTECTED_DACL_SECURITY_INFORMATION,
                },
            },
        };
        let sddl = "O:BAG:SYD:AI(A;OICIID;FA;;;SY)(A;OICIID;FA;;;BA)(A;OICIIOID;GA;;;CO)(A;OICIID;0x1200a9;;;BU)(A;CIID;0x116;;;BU)";
        let descriptor: Vec<u16> = sddl.encode_utf16().chain(Some(0)).collect();
        let path: Vec<u16> = self
            .root
            .join("state")
            .as_os_str()
            .encode_wide()
            .chain(Some(0))
            .collect();
        let mut security = PSECURITY_DESCRIPTOR::default();
        unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                PCWSTR(descriptor.as_ptr()),
                SDDL_REVISION_1,
                &mut security,
                None,
            )
        }
        .expect("SETUP BLOCKED: create official state DACL fixture");
        let result = unsafe {
            SetFileSecurityW(
                PCWSTR(path.as_ptr()),
                OWNER_SECURITY_INFORMATION
                    | DACL_SECURITY_INFORMATION
                    | UNPROTECTED_DACL_SECURITY_INFORMATION,
                security,
            )
        };
        assert!(unsafe { LocalFree(Some(HLOCAL(security.0))) }.0.is_null());
        assert!(result.as_bool(), "SETUP BLOCKED: apply official state DACL");
    }

    fn request(&self) -> Request<'_> {
        Request {
            manifest: &self.manifest,
            installation_id: INSTALLATION,
            target_key: KEY,
            current_build: BUILD,
            product_code: &self.product_code,
        }
    }

    fn layout(&self) -> TestLayout<'_> {
        TestLayout {
            agent: &self.root,
            identity: self.root.join("identity.toml"),
            config: self.root.join("config.toml"),
            registered_source: &self.package,
        }
    }

    fn preserved(&self) -> Vec<Vec<u8>> {
        ["state/agent.json", "identity.toml", "config.toml"]
            .iter()
            .map(|p| std::fs::read(self.root.join(p)).unwrap())
            .collect()
    }
}

#[test]
#[ignore = "Win10 owner RED; isolated fixture, formal SOS inputs, no installer/service/network"]
fn t01_producer_formal_exact_build_publishes_schema1() {
    let f = Fixture::new();
    let before = f.preserved();
    let result = deliver_at(f.request(), f.layout());
    assert!(
        f.preserved() == before,
        "identity/state/config bytes must survive delivery"
    );
    assert_eq!(
        result,
        Ok(()),
        "BUSINESS RED: legitimate signed input must publish approval"
    );
    let bytes = std::fs::read(f.root.join("release-cache/first-install.json")).unwrap();
    assert!(bytes.len() <= 262144);
    let approval: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(approval.as_object().unwrap().len(), 3);
    assert_eq!(approval["schema"], 1);
    assert_eq!(approval["product_code"], f.product_code);
    assert_eq!(
        approval["manifest"],
        serde_json::from_slice::<serde_json::Value>(&f.manifest).unwrap()
    );
    assert_eq!(
        deliver_at(f.request(), f.layout()),
        Ok(()),
        "identical binding is idempotent"
    );
    assert_eq!(
        std::fs::read(f.root.join("release-cache/first-install.json")).unwrap(),
        bytes
    );
}

#[test]
#[ignore = "Win10 owner RED; isolated old approval and identity preservation"]
fn t01_producer_rejects_binding_failures_preserves_old_bytes() {
    let f = Fixture::new();
    let old = b"old approval bytes must not be overwritten";
    std::fs::write(f.root.join("release-cache/first-install.json"), old).unwrap();
    let before = f.preserved();
    let mut failures = Vec::new();
    for case in ["build", "target", "instance", "signature", "product_code"] {
        let mut request = f.request();
        let mut changed: serde_json::Value = serde_json::from_slice(&f.manifest).unwrap();
        changed["targets"][KEY]["sha256"] = "0".repeat(64).into();
        let changed = serde_json::to_vec(&changed).unwrap();
        let expected = match case {
            "build" => {
                request.current_build += 1;
                DeliveryError::BuildMismatch
            }
            "target" => {
                request.target_key = "windows-x86_64-exe-sos";
                DeliveryError::TargetMismatch
            }
            "instance" => {
                request.installation_id = "dddddddddddddddddddddddddddddddd";
                DeliveryError::IdentityMismatch
            }
            "signature" => {
                request.manifest = &changed;
                DeliveryError::SignatureInvalid
            }
            _ => {
                request.product_code = "{00000000-0000-0000-0000-000000000000}";
                DeliveryError::RegisteredSourceMismatch
            }
        };
        let result = deliver_at(request, f.layout());
        assert!(
            f.preserved() == before,
            "failure must preserve identity/state/config bytes"
        );
        assert_eq!(
            std::fs::read(f.root.join("release-cache/first-install.json")).unwrap(),
            old
        );
        if result != Err(expected) {
            failures.push(format!("{case}: {result:?}, expected {expected:?}"));
        }
    }
    assert!(failures.is_empty(), "BUSINESS RED: {}", failures.join("; "));
}

#[test]
#[ignore = "Win10 owner RED; local marker gate and old data preservation"]
fn t01_producer_local_gates_preserve_old_approval() {
    let f = Fixture::new();
    let old = b"previous approved input";
    std::fs::write(f.root.join("release-cache/first-install.json"), old).unwrap();
    let before = f.preserved();
    let mut failures = Vec::new();
    for marker in ["disable-agent", "disable-recovery", "maintenance.json"] {
        let path = f.root.join("state").join(marker);
        std::fs::write(&path, b"blocked").unwrap();
        let result = deliver_at(f.request(), f.layout());
        assert_eq!(std::fs::read(&path).unwrap(), b"blocked");
        assert!(f.preserved() == before, "identity/state/config preserved");
        assert_eq!(
            std::fs::read(f.root.join("release-cache/first-install.json")).unwrap(),
            old
        );
        std::fs::remove_file(path).unwrap();
        if result != Err(DeliveryError::LocallyBlocked) {
            failures.push(format!("{marker}: {result:?}"));
        }
    }
    assert!(failures.is_empty(), "BUSINESS RED: {}", failures.join("; "));
}

#[test]
#[ignore = "Win10 owner; atomic publication and final gate fixture"]
fn t01_producer_final_gate_and_existing_approval_preserved() {
    let f = Fixture::new();
    let before = f.preserved();
    let result =
        first_install_approval_delivery::deliver_before_publish(f.request(), f.layout(), || {
            std::fs::write(f.root.join("state/disable-recovery"), b"late stop").unwrap();
        });
    assert_eq!(result, Err(DeliveryError::LocallyBlocked));
    assert!(f.preserved() == before, "identity/state/config preserved");
    assert!(!f.root.join("release-cache/first-install.json").exists());
    std::fs::remove_file(f.root.join("state/disable-recovery")).unwrap();
    let old = b"occupied approval";
    let result =
        first_install_approval_delivery::deliver_before_publish(f.request(), f.layout(), || {
            std::fs::write(f.root.join("release-cache/first-install.json"), old).unwrap();
        });
    assert_eq!(result, Err(DeliveryError::ExistingApproval));
    assert_eq!(
        std::fs::read(f.root.join("release-cache/first-install.json")).unwrap(),
        old
    );
    assert!(f.preserved() == before, "identity/state/config preserved");
}

#[test]
#[ignore = "Win10 owner; exact build must reject newer signed release too"]
fn t01_producer_newer_signed_build_is_not_current_approval() {
    let f = Fixture::new();
    let before = f.preserved();
    let mut request = f.request();
    request.current_build -= 1;
    assert_eq!(
        deliver_at(request, f.layout()),
        Err(DeliveryError::BuildMismatch)
    );
    assert!(f.preserved() == before, "identity/state/config preserved");
    assert!(!f.root.join("release-cache/first-install.json").exists());
}

#[test]
#[ignore = "Win10 owner; actual MSI registration failure, no registry mutation"]
fn t01_producer_real_registration_is_not_metadata_injection() {
    let f = Fixture::new();
    let before = f.preserved();
    let mut request = f.request();
    request.product_code = "{00000000-0000-0000-0000-000000000000}";
    assert_eq!(
        first_install_approval_delivery::deliver_registered_at(request, f.layout()),
        Err(DeliveryError::RegisteredSourceMismatch)
    );
    assert!(f.preserved() == before, "identity/state/config preserved");
    assert!(!f.root.join("release-cache/first-install.json").exists());
}

#[test]
#[ignore = "Win10 owner; existing same-reference bytes remain exact"]
fn t01_producer_same_reference_preserves_original_json_bytes() {
    let f = Fixture::new();
    let approval = serde_json::json!({"schema":1,"product_code":f.product_code,
        "manifest":serde_json::from_slice::<serde_json::Value>(&f.manifest).unwrap()});
    let old = serde_json::to_vec_pretty(&approval).unwrap();
    std::fs::write(f.root.join("release-cache/first-install.json"), &old).unwrap();
    let before = f.preserved();
    assert_eq!(deliver_at(f.request(), f.layout()), Ok(()));
    assert!(f.preserved() == before, "identity/state/config preserved");
    assert_eq!(
        std::fs::read(f.root.join("release-cache/first-install.json")).unwrap(),
        old
    );
}

#[test]
#[ignore = "Win10 owner; user stop, missing state and occupied rescue fail closed"]
fn t01_producer_stopped_missing_state_and_rescue_preserve_inputs() {
    let f = Fixture::new();
    std::fs::write(
        f.root.join("config.toml"),
        b"[options]\nstop-service = 'Y'\n",
    )
    .unwrap();
    let before = f.preserved();
    assert_eq!(
        deliver_at(f.request(), f.layout()),
        Err(DeliveryError::LocallyBlocked)
    );
    assert!(f.preserved() == before, "stopped config bytes preserved");
    std::fs::write(
        f.root.join("config.toml"),
        b"[options]\nstop-service = 'N'\n",
    )
    .unwrap();
    let state = std::fs::read(f.root.join("state/agent.json")).unwrap();
    std::fs::remove_file(f.root.join("state/agent.json")).unwrap();
    let identity = std::fs::read(f.root.join("identity.toml")).unwrap();
    let config = std::fs::read(f.root.join("config.toml")).unwrap();
    assert_eq!(
        deliver_at(f.request(), f.layout()),
        Err(DeliveryError::UntrustedPath)
    );
    assert!(!f.root.join("state/agent.json").exists());
    assert!(
        std::fs::read(f.root.join("identity.toml")).unwrap() == identity,
        "identity preserved"
    );
    assert!(
        std::fs::read(f.root.join("config.toml")).unwrap() == config,
        "config preserved"
    );
    std::fs::write(f.root.join("state/agent.json"), state).unwrap();
    workspace::create(&f.root.join("rescue")).unwrap();
    workspace::create(&f.root.join("rescue/current")).unwrap();
    std::fs::write(
        f.root.join("rescue/current/bundle.json"),
        b"existing rescue",
    )
    .unwrap();
    let before = f.preserved();
    assert_eq!(
        deliver_at(f.request(), f.layout()),
        Err(DeliveryError::Busy)
    );
    assert!(f.preserved() == before, "identity/state/config preserved");
    assert_eq!(
        std::fs::read(f.root.join("rescue/current/bundle.json")).unwrap(),
        b"existing rescue"
    );
    assert!(!f.root.join("release-cache/first-install.json").exists());
}

fn acl_stage(stage: &str) {
    use std::io::Write;
    println!("T01_ACL_STAGE {stage}");
    std::io::stdout().flush().expect("ACL stage stdout flush");
}

#[test]
#[ignore = "Win10 owner; installed reader rejects writable state, owner and lock conflicts"]
fn t01_installed_state_negative_and_lock_preservation() {
    use std::os::windows::{ffi::OsStrExt, fs::OpenOptionsExt};
    use windows::{
        core::{PCWSTR, PWSTR},
        Win32::{
            Foundation::{CloseHandle, LocalFree, HANDLE, HLOCAL},
            Security::{
                Authorization::{
                    ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW,
                    GetNamedSecurityInfoW, SDDL_REVISION_1, SE_FILE_OBJECT,
                },
                GetTokenInformation, SetFileSecurityW, TokenUser, DACL_SECURITY_INFORMATION,
                OWNER_SECURITY_INFORMATION, PROTECTED_DACL_SECURITY_INFORMATION,
                PSECURITY_DESCRIPTOR, PSID, TOKEN_QUERY, TOKEN_USER,
            },
            System::Threading::{GetCurrentProcess, OpenProcessToken},
        },
    };
    fn acl(path: &Path, text: &str) {
        let wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
        let text: Vec<u16> = text.encode_utf16().chain(Some(0)).collect();
        let mut sd = PSECURITY_DESCRIPTOR::default();
        unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                PCWSTR(text.as_ptr()),
                SDDL_REVISION_1,
                &mut sd,
                None,
            )
        }
        .unwrap();
        let result = unsafe {
            SetFileSecurityW(
                PCWSTR(wide.as_ptr()),
                DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
                sd,
            )
        };
        let error = std::io::Error::last_os_error();
        assert!(unsafe { LocalFree(Some(HLOCAL(sd.0))) }.0.is_null());
        assert!(
            result.as_bool(),
            "SETUP BLOCKED: SetFileSecurityW DACL path={path:?}: {error}"
        );
    }
    fn sid_text(sid: PSID) -> String {
        let mut text = PWSTR::null();
        unsafe { ConvertSidToStringSidW(sid, &mut text) }
            .expect("SETUP BLOCKED: ConvertSidToStringSidW");
        let value = unsafe { text.to_string() }.unwrap();
        assert!(unsafe { LocalFree(Some(HLOCAL(text.0.cast()))) }
            .0
            .is_null());
        value
    }
    fn current_user_sid() -> String {
        let mut token = HANDLE::default();
        unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) }
            .expect("SETUP BLOCKED: OpenProcessToken");
        let mut size = 0;
        let probe = unsafe { GetTokenInformation(token, TokenUser, None, 0, &mut size) };
        assert!(probe.is_err() && size >= std::mem::size_of::<TOKEN_USER>() as u32);
        let mut buffer = vec![
            0usize;
            (size as usize + std::mem::size_of::<usize>() - 1)
                / std::mem::size_of::<usize>()
        ];
        let result = unsafe {
            GetTokenInformation(
                token,
                TokenUser,
                Some(buffer.as_mut_ptr().cast()),
                size,
                &mut size,
            )
        };
        unsafe { CloseHandle(token) }.unwrap();
        result.expect("SETUP BLOCKED: GetTokenInformation TokenUser");
        let user = unsafe { &*buffer.as_ptr().cast::<TOKEN_USER>() };
        let sid = sid_text(user.User.Sid);
        assert!(
            sid != "S-1-5-18" && sid != "S-1-5-32-544",
            "SETUP BLOCKED: owner must be a real non-SY/BA user"
        );
        sid
    }
    fn owner(path: &Path) -> String {
        let wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
        let mut sid = PSID::default();
        let mut sd = PSECURITY_DESCRIPTOR::default();
        let result = unsafe {
            GetNamedSecurityInfoW(
                PCWSTR(wide.as_ptr()),
                SE_FILE_OBJECT,
                OWNER_SECURITY_INFORMATION,
                Some(&mut sid),
                None,
                None,
                None,
                &mut sd,
            )
        };
        assert_eq!(
            result.0, 0,
            "SETUP BLOCKED: GetNamedSecurityInfoW owner path={path:?}"
        );
        let value = sid_text(sid);
        assert!(unsafe { LocalFree(Some(HLOCAL(sd.0))) }.0.is_null());
        value
    }
    fn create_directory_with_acl(path: &Path, text: &str) {
        use std::mem::size_of;
        #[repr(C)]
        struct SecurityAttributes {
            length: u32,
            descriptor: *mut std::ffi::c_void,
            inherit: i32,
        }
        #[link(name = "kernel32")]
        extern "system" {
            fn CreateDirectoryW(path: *const u16, attrs: *const SecurityAttributes) -> i32;
        }
        let wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
        let text: Vec<u16> = text.encode_utf16().chain(Some(0)).collect();
        let mut sd = PSECURITY_DESCRIPTOR::default();
        unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                PCWSTR(text.as_ptr()),
                SDDL_REVISION_1,
                &mut sd,
                None,
            )
        }
        .expect("SETUP BLOCKED: create directory descriptor");
        let attrs = SecurityAttributes {
            length: size_of::<SecurityAttributes>() as u32,
            descriptor: sd.0,
            inherit: 0,
        };
        let result = unsafe { CreateDirectoryW(wide.as_ptr(), &attrs) };
        let error = std::io::Error::last_os_error();
        assert!(unsafe { LocalFree(Some(HLOCAL(sd.0))) }.0.is_null());
        assert!(
            result != 0,
            "SETUP BLOCKED: CreateDirectoryW ACL fixture: {error}"
        );
    }
    fn replace_state(f: &Fixture, sddl: &str) {
        let state = f.root.join("state");
        std::fs::remove_dir_all(&state).expect("SETUP BLOCKED: replace isolated state");
        create_directory_with_acl(&state, sddl);
        std::fs::write(state.join("agent.json"),serde_json::to_vec(&serde_json::json!({"agent_installation_id":INSTALLATION,"target_key":KEY,"management_agent_enabled":true,"disabled":false})).unwrap()).unwrap();
        std::fs::write(state.join("service-recovery-enabled"), b"enabled").unwrap();
    }
    for mode in [
        "delete_child",
        "owner",
        "write_file",
        "lock_failed",
        "cache_readonly",
        "reparse",
    ] {
        let f = Fixture::new();
        let before = f.preserved();
        let old = b"old approval";
        let sidecar = f.root.join("release-cache/first-install.json");
        std::fs::write(&sidecar, old).unwrap();
        let state = f.root.join("state");
        let file = state.join("agent.json");
        let mut held = None;
        match mode {
            "delete_child" => replace_state(
                &f,
                "O:BAG:BAD:(A;OICI;FA;;;SY)(A;OICI;FA;;;BA)(A;CI;0x156;;;BU)",
            ),
            "owner" => {
                let sid = current_user_sid();
                replace_state(
                    &f,
                    &format!("O:{sid}G:BAD:(A;OICI;FA;;;SY)(A;OICI;FA;;;BA)"),
                );
                assert_eq!(
                    owner(&state),
                    sid,
                    "SETUP BLOCKED: created owner differs from token user"
                );
                println!("T01_NEGATIVE owner=current-token-user; non_SY_BA=true");
            }
            "write_file" => acl(&file, "O:BAD:P(A;;FA;;;SY)(A;;FA;;;BA)(A;;FW;;;BU)"),
            "lock_failed" => {
                held = Some(
                    std::fs::OpenOptions::new()
                        .read(true)
                        .share_mode(0)
                        .open(&file)
                        .unwrap(),
                )
            }
            "cache_readonly" => acl(
                &f.root.join("release-cache"),
                "O:BAD:P(A;OICI;FA;;;SY)(A;OICI;FA;;;BA)(A;OICI;FR;;;BU)",
            ),
            _ => {
                std::fs::rename(&file, state.join("saved.json")).unwrap();
                std::os::windows::fs::symlink_file(state.join("saved.json"), &file).expect(
                    "SETUP BLOCKED: symlink permission required; no reparse coverage without it",
                );
            }
        }
        let result = deliver_at(f.request(), f.layout());
        drop(held);
        assert!(
            f.preserved() == before,
            "negative case retains state/identity/config"
        );
        assert_eq!(std::fs::read(&sidecar).unwrap(), old);
        assert_eq!(result, Err(DeliveryError::UntrustedPath), "case={mode}");
        println!("T01_NEGATIVE case={mode}; rejected=UntrustedPath; preserved=true");
    }
    let f = Fixture::new();
    let before = f.preserved();
    let mut observed = false;
    let result =
        first_install_approval_delivery::deliver_before_publish(f.request(), f.layout(), || {
            for path in [f.root.join("state/agent.json"), f.root.join("config.toml")] {
                let denied = std::fs::OpenOptions::new()
                    .write(true)
                    .share_mode(3)
                    .open(&path)
                    .unwrap_err();
                assert_eq!(denied.raw_os_error(), Some(32));
                assert!(std::fs::File::open(&path).is_ok());
            }
            for path in [&f.root, &f.root.join("state")] {
                let denied = std::fs::OpenOptions::new()
                    .access_mode(0x10000)
                    .share_mode(7)
                    .custom_flags(0x02000000)
                    .open(path)
                    .unwrap_err();
                assert_eq!(denied.raw_os_error(), Some(32));
            }
            observed = true;
        });
    assert!(observed);
    assert_eq!(result, Ok(()));
    assert!(f.preserved() == before, "held handles preserve prior bytes");
}

#[test]
#[ignore = "VM owner; installed state inheritance contract candidate, no production ACL mutation"]
fn t01_producer_installed_state_inheritance_readonly_candidate() {
    use std::os::windows::ffi::OsStrExt;
    use windows::{
        core::PCWSTR,
        Win32::{
            Foundation::{LocalFree, HLOCAL},
            Security::{
                Authorization::{
                    ConvertStringSecurityDescriptorToSecurityDescriptorW, GetNamedSecurityInfoW,
                    SDDL_REVISION_1, SE_FILE_OBJECT,
                },
                SetFileSecurityW, DACL_SECURITY_INFORMATION, OWNER_SECURITY_INFORMATION,
                PSECURITY_DESCRIPTOR, UNPROTECTED_DACL_SECURITY_INFORMATION,
            },
        },
    };
    fn descriptor(path: &Path) -> Vec<u8> {
        #[link(name = "advapi32")]
        extern "system" {
            fn GetSecurityDescriptorLength(sd: *const std::ffi::c_void) -> u32;
        }
        let wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
        let mut sd = PSECURITY_DESCRIPTOR::default();
        assert_eq!(
            unsafe {
                GetNamedSecurityInfoW(
                    PCWSTR(wide.as_ptr()),
                    SE_FILE_OBJECT,
                    OWNER_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION,
                    None,
                    None,
                    None,
                    None,
                    &mut sd,
                )
            }
            .0,
            0
        );
        let size = unsafe { GetSecurityDescriptorLength(sd.0) };
        assert!(size > 0 && size < 65536);
        let bytes =
            unsafe { std::slice::from_raw_parts(sd.0.cast::<u8>(), size as usize) }.to_vec();
        assert!(unsafe { LocalFree(Some(HLOCAL(sd.0))) }.0.is_null());
        bytes
    }
    fn set_inherited_acl(path: &Path, sddl: &str) {
        let wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
        let text: Vec<u16> = sddl.encode_utf16().chain(Some(0)).collect();
        let mut sd = PSECURITY_DESCRIPTOR::default();
        unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                PCWSTR(text.as_ptr()),
                SDDL_REVISION_1,
                &mut sd,
                None,
            )
        }
        .unwrap();
        let applied = unsafe {
            SetFileSecurityW(
                PCWSTR(wide.as_ptr()),
                DACL_SECURITY_INFORMATION | UNPROTECTED_DACL_SECURITY_INFORMATION,
                sd,
            )
        };
        let error = std::io::Error::last_os_error();
        assert!(unsafe { LocalFree(Some(HLOCAL(sd.0))) }.0.is_null());
        assert!(
            applied.as_bool(),
            "SETUP BLOCKED: SetFileSecurityW installed inherited ACL path={path:?}: {error}"
        );
    }
    let f = Fixture::new();
    let state = f.root.join("state");
    let approval = f.root.join("release-cache/first-install.json");
    let old = serde_json::to_vec_pretty(
        &serde_json::json!({"schema":1,"product_code":f.product_code,
        "manifest":serde_json::from_slice::<serde_json::Value>(&f.manifest).unwrap()}),
    )
    .unwrap();
    std::fs::write(&approval, &old).unwrap();
    let before = f.preserved();
    let marker = std::fs::read(state.join("service-recovery-enabled")).unwrap();
    let root_sddl = "O:SYG:SYD:AI(A;OICIID;FA;;;SY)(A;OICIID;FA;;;BA)(A;OICIIOID;GA;;;CO)(A;OICIID;0x1200a9;;;BU)(A;CIID;DCLCRPCR;;;BU)";
    let state_sddl = "O:BAG:SYD:AI(A;OICIID;FA;;;SY)(A;OICIID;FA;;;BA)(A;OICIIOID;GA;;;CO)(A;OICIID;0x1200a9;;;BU)(A;CIID;0x116;;;BU)";
    set_inherited_acl(&f.root, root_sddl);
    set_inherited_acl(&state, state_sddl);
    let root_sd = descriptor(&f.root);
    let state_sd = descriptor(&state);
    let cache_sd = descriptor(&f.root.join("release-cache"));
    let approval_sd = descriptor(&approval);
    for file in [
        state.join("agent.json"),
        state.join("service-recovery-enabled"),
    ] {
        workspace::verify_restore_target(&file, false)
            .expect("SETUP BLOCKED: file contract must remain valid");
    }
    assert!(
        workspace::verify(&f.root).is_err(),
        "strict workspace policy remains distinct"
    );
    workspace::verify(&f.root.join("release-cache")).unwrap();
    workspace::verify_file(&approval).unwrap();
    println!(
        "T01_STATE_ACL strict_directory={:?}",
        workspace::verify(&state)
    );
    let result = deliver_at(f.request(), f.layout());
    assert!(
        f.preserved() == before,
        "state/identity/config bytes retained"
    );
    assert_eq!(std::fs::read(&approval).unwrap(), old);
    assert_eq!(
        std::fs::read(state.join("service-recovery-enabled")).unwrap(),
        marker
    );
    assert_eq!(descriptor(&state), state_sd, "state ACL unchanged");
    assert_eq!(descriptor(&f.root), root_sd, "root ACL unchanged");
    assert_eq!(
        descriptor(&f.root.join("release-cache")),
        cache_sd,
        "cache ACL unchanged"
    );
    assert_eq!(descriptor(&approval), approval_sd, "approval ACL unchanged");
    println!("T01_STATE_ACL producer={result:?}; preserved=true");
    assert_eq!(
        result,
        Ok(()),
        "CONTRACT CANDIDATE: installed state inheritance must permit read-only idempotent delivery"
    );
}

#[test]
#[ignore = "Win10 owner; actual HTTP fast, silent headers and oversized bodies"]
fn t01_http_fast_silent_headers_and_oversize_preserve_data() {
    use first_install_approval_delivery::{
        coordinate_approval, fetch_loopback_fixture, CoordinationOutcome, CurrentInstallation,
    };
    use std::{
        io::{Read, Write},
        net::TcpListener,
        time::{Duration, Instant},
    };
    let f = Fixture::new();
    let sidecar = f.root.join("release-cache/first-install.json");
    let old = b"existing approval retained by HTTP boundary tests";
    std::fs::write(&sidecar, old).unwrap();
    let before = f.preserved();
    for mode in [
        "fast",
        "silent_headers",
        "oversize_length",
        "oversize_chunked",
        "headers_then_slow_body",
    ] {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let address = listener.local_addr().unwrap();
        let body = f.manifest.clone();
        let server = std::thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_secs(5);
            let mut stream = loop {
                match listener.accept() {
                    Ok((stream, _)) => break stream,
                    Err(e)
                        if e.kind() == std::io::ErrorKind::WouldBlock
                            && Instant::now() < deadline =>
                    {
                        std::thread::sleep(Duration::from_millis(10))
                    }
                    Err(e) => panic!("HTTP fixture accept: {e}"),
                }
            };
            stream
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            stream
                .set_write_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            stream.set_nodelay(true).unwrap();
            let mut request = Vec::new();
            while !request.ends_with(b"\r\n\r\n") && request.len() < 8192 {
                let mut byte = [0];
                if stream.read(&mut byte).unwrap_or(0) == 0 {
                    return false;
                }
                request.push(byte[0]);
            }
            assert!(request.starts_with(b"GET /rd/update/v1/manifest/stable.json HTTP/1.1\r\n"));
            if mode == "silent_headers" {
                std::thread::sleep(Duration::from_millis(1000));
                return true;
            }
            if mode == "oversize_length" {
                let _ = stream.write_all(
                    b"HTTP/1.1 200 OK\r\nContent-Length: 262145\r\nConnection: close\r\n\r\n",
                );
                return true;
            }
            if mode == "oversize_chunked" {
                let _ = stream.write_all(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n40001\r\n")
                    .and_then(|_| stream.write_all(&vec![b' '; 262145]))
                    .and_then(|_| stream.write_all(b"\r\n0\r\n\r\n"));
                return true;
            }
            if mode == "headers_then_slow_body" {
                std::thread::sleep(Duration::from_millis(250));
            }
            if stream
                .write_all(
                    format!(
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        body.len()
                    )
                    .as_bytes(),
                )
                .is_err()
            {
                return true;
            }
            if mode == "headers_then_slow_body" {
                std::thread::sleep(Duration::from_millis(450));
            }
            let _ = stream.write_all(&body);
            true
        });
        let delivered = std::cell::Cell::new(0);
        let start = Instant::now();
        let result = coordinate_approval(
            CurrentInstallation {
                installation_id: INSTALLATION,
                target_key: KEY,
                current_build: BUILD,
                product_code: &f.product_code,
            },
            || fetch_loopback_fixture(address, Duration::from_millis(400)),
            |_| {
                delivered.set(delivered.get() + 1);
                Ok(())
            },
        );
        let elapsed = start.elapsed();
        assert!(
            server.join().unwrap(),
            "SETUP BLOCKED: request not received"
        );
        assert!(
            f.preserved() == before,
            "HTTP errors preserve identity/state/config"
        );
        assert_eq!(std::fs::read(&sidecar).unwrap(), old);
        println!(
            "T01_HTTP_CASE {mode} result={result:?} elapsed_ms={} delivered={}",
            elapsed.as_millis(),
            delivered.get()
        );
        assert_eq!(
            result,
            Ok(if mode == "fast" {
                CoordinationOutcome::Delivered
            } else {
                CoordinationOutcome::WaitingSource
            })
        );
        assert_eq!(delivered.get(), if mode == "fast" { 1 } else { 0 });
        assert!(
            elapsed < Duration::from_millis(1200),
            "HTTP total budget exceeded"
        );
    }
}

#[test]
#[ignore = "Win10 owner HTTP budget RED; real loopback slow chunked response"]
fn t01_http_slow_chunks_exhaust_total_budget_preserve_old_data() {
    use first_install_approval_delivery::{
        coordinate_approval, fetch_loopback_fixture, CoordinationOutcome, CurrentInstallation,
    };
    use std::{
        io::{Read, Write},
        net::TcpListener,
        sync::{
            atomic::{AtomicBool, AtomicUsize, Ordering},
            Arc,
        },
        time::{Duration, Instant},
    };
    let f = Fixture::new();
    let sidecar = f.root.join("release-cache/first-install.json");
    let old = b"previous approval survives timeout";
    std::fs::write(&sidecar, old).unwrap();
    let before = f.preserved();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let address = listener.local_addr().unwrap();
    let stop = Arc::new(AtomicBool::new(false));
    let sent = Arc::new(AtomicUsize::new(0));
    let server_stop = stop.clone();
    let server_sent = sent.clone();
    let body = f.manifest.clone();
    let server = std::thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(8);
        let mut stream = loop {
            match listener.accept() {
                Ok((stream, _)) => break stream,
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    if server_stop.load(Ordering::SeqCst) || Instant::now() >= deadline {
                        return;
                    }
                    std::thread::sleep(Duration::from_millis(10));
                }
                Err(e) => panic!("fixture accept failed: {e}"),
            }
        };
        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        stream
            .set_write_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        stream.set_nodelay(true).unwrap();
        let mut request = Vec::new();
        while !request.ends_with(b"\r\n\r\n") && request.len() < 8192 {
            let mut byte = [0u8];
            if stream.read(&mut byte).unwrap_or(0) == 0 {
                return;
            }
            request.push(byte[0]);
        }
        assert!(request.starts_with(b"GET /rd/update/v1/manifest/stable.json HTTP/1.1\r\n"));
        if stream
            .write_all(
                b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n",
            )
            .is_err()
        {
            return;
        }
        let chunk_size = (body.len() + 19) / 20;
        for chunk in body.chunks(chunk_size) {
            if server_stop.load(Ordering::SeqCst) {
                return;
            }
            let header = format!("{:x}\r\n", chunk.len());
            if stream
                .write_all(header.as_bytes())
                .and_then(|_| stream.write_all(chunk))
                .and_then(|_| stream.write_all(b"\r\n"))
                .and_then(|_| stream.flush())
                .is_err()
            {
                return;
            }
            server_sent.fetch_add(1, Ordering::SeqCst);
            std::thread::sleep(Duration::from_millis(100));
        }
        let _ = stream.write_all(b"0\r\n\r\n");
    });
    let delivered = std::cell::Cell::new(0);
    let start = Instant::now();
    let result = coordinate_approval(
        CurrentInstallation {
            installation_id: INSTALLATION,
            target_key: KEY,
            current_build: BUILD,
            product_code: &f.product_code,
        },
        || fetch_loopback_fixture(address, Duration::from_millis(400)),
        |_| {
            delivered.set(delivered.get() + 1);
            Ok(())
        },
    );
    let elapsed = start.elapsed();
    stop.store(true, Ordering::SeqCst);
    server.join().unwrap();
    assert!(
        f.preserved() == before,
        "identity/state/config preserved after slow body"
    );
    assert_eq!(std::fs::read(&sidecar).unwrap(), old);
    println!(
        "T01_HTTP_BUDGET result={result:?} elapsed_ms={} chunks={} delivered={}",
        elapsed.as_millis(),
        sent.load(Ordering::SeqCst),
        delivered.get()
    );
    assert!(
        sent.load(Ordering::SeqCst) >= 2,
        "SETUP BLOCKED: slow response was not exercised"
    );
    assert_eq!(
        result,
        Ok(CoordinationOutcome::WaitingSource),
        "BUSINESS RED: slow body must exhaust total budget"
    );
    assert_eq!(
        delivered.get(),
        0,
        "timed-out source must not reach producer"
    );
    assert!(
        elapsed <= Duration::from_millis(1200),
        "total 400ms budget plus 800ms scheduling allowance exceeded"
    );
}

#[test]
#[ignore = "Win10 owner; coordination RED with production signature and real file producer"]
fn t01_coordination_exact_public_manifest_delivers_before_return() {
    use first_install_approval_delivery::{
        coordinate_approval, CoordinationOutcome, CurrentInstallation,
    };
    let f = Fixture::new();
    let before = f.preserved();
    let fetched = std::cell::Cell::new(0);
    let delivered = std::cell::Cell::new(0);
    let result = coordinate_approval(
        CurrentInstallation {
            installation_id: INSTALLATION,
            target_key: KEY,
            current_build: BUILD,
            product_code: &f.product_code,
        },
        || {
            fetched.set(fetched.get() + 1);
            Ok(f.manifest.clone())
        },
        |request| {
            delivered.set(delivered.get() + 1);
            deliver_at(request, f.layout())
        },
    );
    assert!(
        f.preserved() == before,
        "coordination preserves identity/state/config"
    );
    println!(
        "T01_COORD_RESULT {result:?}; fetch={}; deliver={}",
        fetched.get(),
        delivered.get()
    );
    assert_eq!(
        result,
        Ok(CoordinationOutcome::Delivered),
        "BUSINESS RED: exact signed public approval must pass through coordination to producer"
    );
    assert_eq!(fetched.get(), 1);
    assert_eq!(delivered.get(), 1);
    let approval: serde_json::Value = serde_json::from_slice(
        &std::fs::read(f.root.join("release-cache/first-install.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(
        approval["manifest"],
        serde_json::from_slice::<serde_json::Value>(&f.manifest).unwrap()
    );
    assert_eq!(approval["schema"], 1);
}

#[test]
#[ignore = "Win10 owner; unavailable or mismatched approval must not invoke producer"]
fn t01_coordination_unavailable_or_nonexact_keeps_waiting_and_old_bytes() {
    use first_install_approval_delivery::{
        coordinate_approval, CoordinationOutcome, CurrentInstallation,
    };
    let f = Fixture::new();
    let old = b"previous approval";
    let sidecar = f.root.join("release-cache/first-install.json");
    std::fs::write(&sidecar, old).unwrap();
    let before = f.preserved();
    let mut failures = Vec::new();
    for mode in ["network", "newer", "older", "signature"] {
        let delivered = std::cell::Cell::new(0);
        let current_build = match mode {
            "newer" => BUILD - 1,
            "older" => BUILD + 1,
            _ => BUILD,
        };
        let result = coordinate_approval(
            CurrentInstallation {
                installation_id: INSTALLATION,
                target_key: KEY,
                current_build,
                product_code: &f.product_code,
            },
            || {
                if mode == "network" {
                    return Err("bounded source unavailable".into());
                }
                if mode == "signature" {
                    let mut value: serde_json::Value = serde_json::from_slice(&f.manifest).unwrap();
                    value["targets"][KEY]["sha256"] = "0".repeat(64).into();
                    return Ok(serde_json::to_vec(&value).unwrap());
                }
                Ok(f.manifest.clone())
            },
            |_| {
                delivered.set(delivered.get() + 1);
                Ok(())
            },
        );
        assert!(
            f.preserved() == before,
            "failed source preserves identity/state/config"
        );
        assert_eq!(std::fs::read(&sidecar).unwrap(), old);
        assert_eq!(
            delivered.get(),
            0,
            "invalid source must not invoke producer"
        );
        let expected = match mode {
            "network" => CoordinationOutcome::WaitingSource,
            "signature" => CoordinationOutcome::WaitingSignature,
            _ => CoordinationOutcome::WaitingExactBuild,
        };
        if result != Ok(expected) {
            failures.push(format!("{mode}: {result:?}"));
        }
    }
    assert!(failures.is_empty(), "BUSINESS RED: {}", failures.join("; "));
}

fn approval_users_readonly_acl(path: &Path) {
    use std::os::windows::ffi::OsStrExt;
    use windows::{
        core::PCWSTR,
        Win32::{
            Foundation::{LocalFree, HLOCAL},
            Security::{
                Authorization::{
                    ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1,
                },
                SetFileSecurityW, DACL_SECURITY_INFORMATION, OWNER_SECURITY_INFORMATION,
                PROTECTED_DACL_SECURITY_INFORMATION, PSECURITY_DESCRIPTOR,
            },
        },
    };
    let sddl: Vec<u16> = "O:BAD:P(A;;FA;;;SY)(A;;FA;;;BA)(A;;FR;;;BU)"
        .encode_utf16()
        .chain(Some(0))
        .collect();
    let mut descriptor = PSECURITY_DESCRIPTOR::default();
    acl_stage("02_descriptor_begin");
    unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            PCWSTR(sddl.as_ptr()),
            SDDL_REVISION_1,
            &mut descriptor,
            None,
        )
        .expect("SETUP BLOCKED: construct real approval ACL");
    }
    acl_stage("03_descriptor_ok");
    let wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    let applied = unsafe {
        SetFileSecurityW(
            PCWSTR(wide.as_ptr()),
            OWNER_SECURITY_INFORMATION
                | DACL_SECURITY_INFORMATION
                | PROTECTED_DACL_SECURITY_INFORMATION,
            descriptor,
        )
    };
    acl_stage("04_set_file_security_returned");
    let freed = unsafe { LocalFree(Some(HLOCAL(descriptor.0))) };
    acl_stage("05_descriptor_freed");
    assert!(
        freed.0.is_null(),
        "SETUP BLOCKED: security descriptor cleanup"
    );
    assert!(applied.as_bool(), "SETUP BLOCKED: apply real approval ACL");
}

#[test]
#[ignore = "Win10 owner ACL RED; real Users-readonly ACL, no implementation fix yet"]
fn t01_producer_existing_users_readonly_acl_refused_preserves_bytes() {
    acl_stage("00_fixture_begin");
    let f = Fixture::new();
    acl_stage("01_fixture_ok");
    let approval = serde_json::json!({"schema":1,"product_code":f.product_code,
        "manifest":serde_json::from_slice::<serde_json::Value>(&f.manifest).unwrap()});
    let old = serde_json::to_vec_pretty(&approval).unwrap();
    let path = f.root.join("release-cache/first-install.json");
    std::fs::write(&path, &old).unwrap();
    acl_stage("01b_old_approval_written");
    approval_users_readonly_acl(&path);
    acl_stage("06_acl_applied");
    workspace::verify_restore_target(&path, false)
        .expect("SETUP BLOCKED: ACL must reproduce weaker producer acceptance");
    acl_stage("07_weak_acl_accepted");
    assert_eq!(
        workspace::verify_file(&path).unwrap_err(),
        "workspace unexpected permission principal",
        "SETUP BLOCKED: strict reader must reject this real ACL"
    );
    acl_stage("08_strict_acl_refused");
    let before = f.preserved();
    acl_stage("09_producer_begin");
    let result = deliver_at(f.request(), f.layout());
    acl_stage("10_producer_returned");
    println!("T01_ACL_RESULT {result:?}");
    {
        use std::io::Write;
        std::io::stdout().flush().unwrap();
    }
    assert!(
        f.preserved() == before,
        "identity/state/config bytes preserved"
    );
    assert_eq!(
        std::fs::read(&path).unwrap(),
        old,
        "existing approval bytes preserved"
    );
    assert_eq!(
        workspace::verify_file(&path).unwrap_err(),
        "workspace unexpected permission principal",
        "existing approval ACL must not be repaired"
    );
    acl_stage("11_preservation_verified");
    assert_eq!(
        result,
        Err(DeliveryError::UntrustedPath),
        "BUSINESS RED: existing approval must satisfy the reader's strict file ACL"
    );
}
