#[path = "../../src/platform/windows_system_dirs.rs"]
#[allow(dead_code)]
mod system_dirs;
// 代理源码里有 `crate::management_agent::system_dirs` 的写法，测试入口补同名路径。
#[allow(unused_imports)]
mod management_agent {
    pub(crate) use super::system_dirs;
}
#[path = "../../src/management_agent/agent_restore.rs"]
mod agent_restore;
#[path = "../../src/management_agent/bootstrap_update.rs"]
mod bootstrap_update;
#[path = "../../src/management_agent/directory_backup.rs"]
mod directory_backup;
#[path = "../../src/management_agent/file_restore.rs"]
mod file_restore;
#[path = "../../src/management_agent/handoff.rs"]
mod handoff;
#[path = "../../src/management_agent/health.rs"]
mod health;
#[path = "../../src/management_agent/health_windows.rs"]
mod health_windows;
#[path = "../../src/management_agent/helper_runtime.rs"]
mod helper_runtime;
#[path = "../../src/management_agent/identity.rs"]
mod identity;
#[path = "../../src/management_agent/install_execution.rs"]
mod install_execution;
#[path = "../../src/platform/windows_agent_guard.rs"]
mod install_guard;
#[path = "../../src/management_agent/install_plan.rs"]
mod install_plan;
#[path = "../../src/management_agent/installer_process.rs"]
mod installer_process;
#[path = "../../src/management_agent/local_events.rs"]
mod local_events;
#[path = "../../src/management_agent/local_recovery_events.rs"]
mod local_recovery_events;
#[path = "../../src/management_agent/local_repair.rs"]
mod local_repair;
#[path = "../../src/management_agent/local_repair_plan.rs"]
mod local_repair_plan;
#[path = "../../src/management_agent/local_repair_schedule.rs"]
mod local_repair_schedule;
#[path = "../../src/management_agent/maintenance.rs"]
mod maintenance;
#[path = "../../src/management_agent/msi_assets.rs"]
mod msi_assets;
#[path = "../../src/management_agent/msi_metadata.rs"]
mod msi_metadata;
#[path = "../../src/management_agent/native_install.rs"]
mod native_install;
#[path = "../../src/management_agent/package_gate.rs"]
mod package_gate;
#[path = "../../src/management_agent/parent_handoff.rs"]
mod parent_handoff;
#[path = "../../src/management_agent/preparation.rs"]
mod preparation;
#[path = "../../src/management_agent/purge_approval.rs"]
mod purge_approval;
#[path = "../../src/management_agent/purge_discovery.rs"]
mod purge_discovery;
#[path = "../../src/management_agent/purge_handoff.rs"]
mod purge_handoff;
#[path = "../../src/management_agent/purge_policy.rs"]
mod purge_policy;
#[path = "../../src/management_agent/purge_preflight.rs"]
mod purge_preflight;
#[path = "../../src/management_agent/purge_service.rs"]
mod purge_service;
#[path = "../../src/management_agent/recovery.rs"]
mod recovery;
#[path = "../../src/management_agent/rescue_assets.rs"]
mod rescue_assets;
#[path = "../../src/management_agent/rescue_recovery.rs"]
mod rescue_recovery;
#[path = "../../src/management_agent/rescue_status.rs"]
mod rescue_status;
#[path = "../../src/management_agent/resume_task.rs"]
mod resume_task;
#[path = "../../src/management_agent/service_control.rs"]
mod service_control;
#[path = "../../src/management_agent/state_store.rs"]
mod state_store;
#[path = "../../src/management_agent/transaction.rs"]
mod transaction;
#[path = "../../src/management_agent/workspace.rs"]
mod workspace;
