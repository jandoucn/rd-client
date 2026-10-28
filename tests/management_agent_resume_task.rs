// resume_task.rs 引用 `crate::management_agent::system_dirs`，
// 该模块在生产 crate 里只在 windows 下编译。
#![cfg(windows)]
#[path = "../src/platform/windows_system_dirs.rs"]
#[allow(dead_code)]
mod system_dirs;
mod management_agent {
    pub(crate) use super::system_dirs;
}
#[path = "../src/management_agent/resume_task.rs"]
mod resume_task;

#[test]
fn resume_task_is_bound_to_one_transaction_and_fixed_binary_without_freeform_arguments() {
    let id = "a".repeat(32);
    let helper = std::path::Path::new(
        r"C:\ProgramData\RustDesk\ManagementAgent\recovery-runtime\helper.exe",
    );
    let definition = resume_task::definition(helper, &id).unwrap();
    resume_task::verify(&definition, helper, &id).unwrap();
    for text in [
        "<UserId>S-1-5-18</UserId>",
        "<MultipleInstancesPolicy>IgnoreNew</MultipleInstancesPolicy>",
        "<Interval>PT5M</Interval>",
        "<Duration>P1D</Duration>",
        "<BootTrigger>",
        "resume-install --transaction-id",
    ] {
        assert!(definition.contains(text), "{text}");
    }
    for changed in [
        definition.replace("resume-install", "powershell.exe"),
        definition.replace(&id, &"b".repeat(32)),
        definition.replace("IgnoreNew", "Parallel"),
        definition.replace("HighestAvailable", "LeastPrivilege"),
        definition.replace("S-1-5-18", "S-1-5-19"),
        definition.replace(
            "<Enabled>true</Enabled></Settings>",
            "<Enabled>false</Enabled></Settings>",
        ),
        definition.replace("<Exec>", "<Exec/><Exec>"),
    ] {
        assert!(resume_task::verify(&changed, helper, &id).is_err());
    }
    assert!(resume_task::definition(helper, "../escape").is_err());
    assert!(resume_task::definition(std::path::Path::new("relative.exe"), &id).is_err());
}

#[test]
fn scheduler_normalized_default_flags_and_quotes_keep_same_contract() {
    let id = "a".repeat(32);
    let helper = std::path::Path::new(r"C:\ProgramData\RustDesk\recovery runtime\helper.exe");
    let definition = resume_task::definition(helper, &id).unwrap();
    let normalized = definition
        .replace("&quot;", "\"")
        .replace("<Enabled>true</Enabled>", "");
    resume_task::verify(&normalized, helper, &id).unwrap();
    resume_task::verify(
        &normalized.replace("<BootTrigger></BootTrigger>", "<BootTrigger />"),
        helper,
        &id,
    )
    .unwrap();
    assert!(resume_task::verify(
        &normalized.replace(
            "<BootTrigger></BootTrigger>",
            "<BootTrigger /><BootTrigger />"
        ),
        helper,
        &id
    )
    .is_err());
    assert!(resume_task::verify(
        &normalized.replace("<BootTrigger>", "<BootTrigger><Enabled>false</Enabled>"),
        helper,
        &id
    )
    .is_err());
}

#[test]
fn repetition_window_starts_at_registration_time_not_an_expired_constant() {
    let id = "a".repeat(32);
    let helper = std::path::Path::new(r"C:\ProgramData\RustDesk\helper.exe");
    let before = chrono::Utc::now() - chrono::Duration::seconds(2);
    let xml = resume_task::definition(helper, &id).unwrap();
    let start = xml
        .split("<StartBoundary>")
        .nth(1)
        .unwrap()
        .split("</StartBoundary>")
        .next()
        .unwrap();
    let parsed = chrono::DateTime::parse_from_rfc3339(start)
        .unwrap()
        .with_timezone(&chrono::Utc);
    assert!(parsed >= before && parsed <= chrono::Utc::now() + chrono::Duration::seconds(2));
}

#[test]
fn native_disabled_task_roundtrip_is_read_back_and_removed_without_running_action() {
    let id = hex::encode(hbb_common::sodiumoxide::randombytes::randombytes(16));
    let root =
        std::env::temp_dir().join(format!("rustdesk-resume-task-{}-{id}", std::process::id()));
    std::fs::create_dir(&root).unwrap();
    let helper = std::env::var_os("SystemRoot")
        .map(std::path::PathBuf::from)
        .unwrap()
        .join("System32/whoami.exe");
    let definition = resume_task::definition(&helper, &id)
        .unwrap()
        .replace("<Enabled>true</Enabled>", "<Enabled>false</Enabled>");
    resume_task::register_definition(&root, &id, &definition).unwrap();
    let observed = resume_task::read_back(&id).unwrap();
    let removed = resume_task::remove(&id);
    removed.unwrap();
    assert!(observed.contains("<Enabled>false</Enabled>"));
    assert!(resume_task::verify(&observed, &helper, &id).is_err());
    resume_task::verify(
        &observed.replace("<Enabled>false</Enabled>", "<Enabled>true</Enabled>"),
        &helper,
        &id,
    )
    .unwrap();
    assert!(resume_task::read_back(&id).is_err());
    std::fs::remove_dir_all(root).unwrap();
}
