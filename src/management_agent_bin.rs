#[allow(dead_code)]
#[path = "management_agent.rs"]
mod management_agent;

fn main() {
    #[cfg(windows)]
    windows_main();

    #[cfg(not(windows))]
    eprintln!("RustDesk Management Agent is supported on Windows only");
}

#[cfg(windows)]
fn windows_main() {
    let _logger = hbb_common::init_log(false, "management-agent");
    let arguments = std::env::args_os().skip(1).collect::<Vec<_>>();
    let command = arguments
        .first()
        .map(|argument| argument.to_string_lossy().into_owned())
        .unwrap_or_else(|| "--service".to_owned());
    if matches!(
        command.as_str(),
        "local-repair-prepare" | "local-repair-helper" | "local-repair-resume"
    ) {
        if let Err(error) = management_agent::run_local_repair(&arguments[1..], &command) {
            eprintln!("local repair failed: {error}");
            std::process::exit(1);
        }
        return;
    }
    if matches!(
        command.as_str(),
        "exe-transaction-prepare"
            | "exe-transaction-launch"
            | "exe-transaction-helper"
            | "exe-transaction-resume"
            | "exe-selfcheck"
    ) {
        if let Err(error) = management_agent::run_exe_transaction(&arguments[1..], &command) {
            eprintln!("EXE transaction failed: {error}");
            std::process::exit(1);
        }
        return;
    }
    if command == "transaction-helper" {
        if let Err(error) = management_agent::run_transaction_helper(&arguments[1..]) {
            eprintln!("management agent transaction helper failed: {error}");
            std::process::exit(1);
        }
        return;
    }
    if command == "resume-install" {
        if let Err(error) = management_agent::resume_transaction_helper(&arguments[1..]) {
            eprintln!("management agent transaction resume failed: {error}");
            std::process::exit(1);
        }
        return;
    }
    if command == "--management-agent-recovery-helper" {
        if let Err(error) = management_agent::run_recovery_helper(&arguments) {
            eprintln!("management agent recovery helper failed: {error}");
            std::process::exit(1);
        }
        return;
    }
    #[cfg(not(test))]
    if command == "--purge-helper" {
        if arguments.len() != 3 || arguments[1] != "--transaction-id" {
            eprintln!("purge helper accepts only --transaction-id <32hex>");
            std::process::exit(2);
        }
        let id = arguments.get(2).and_then(|value| value.to_str()).unwrap_or("");
        if let Err(error) = management_agent::run_purge_helper(id) {
            eprintln!("purge helper failed: {error}");
            std::process::exit(1);
        }
        return;
    }
    let program_data = std::env::var_os("ProgramData").unwrap_or_else(|| "C:\\ProgramData".into());
    let config = match management_agent::parse_service_arguments(
        arguments
            .iter()
            .skip(1)
            .map(|argument| argument.to_string_lossy().into_owned()),
    ) {
        Ok(config) => config,
        Err(error) => {
            eprintln!("invalid management agent arguments: {error}");
            std::process::exit(2);
        }
    };
    let paths = management_agent::AgentPaths::from_program_data(program_data, config.edition);
    match command.as_str() {
        "install-bootstrap" => {
            if let Err(error) = management_agent::bootstrap::install(&config, &paths) {
                eprintln!("management agent bootstrap install failed: {error}");
                std::process::exit(1);
            }
        }
        "bootstrap" => {
            if let Err(error) = management_agent::bootstrap::run_once(&config, &paths) {
                hbb_common::log::warn!("management agent bootstrap failed: {error}");
                std::process::exit(1);
            }
        }
        "status" if arguments.iter().any(|argument| argument == "--json") => {
            match management_agent::local_status(&paths).and_then(|value| serde_json::to_string(&value).map_err(|error| error.to_string())) {
                Ok(value) => println!("{value}"),
                Err(error) => {
                    eprintln!("management agent status failed: {error}");
                    std::process::exit(1);
                }
            }
        }
        "status" => println!("RustDesk Management Agent root: {}", paths.root.display()),
        "check-now" => {
            if let Err(error) = management_agent::check_now_with_package(
                config.edition,
                config.package_kind,
                &paths,
                config.normal_install_path.as_deref(),
            ) {
                eprintln!("management agent check failed: {error}");
                std::process::exit(1);
            }
        }
        #[cfg(not(test))]
        "uninstall" if std::env::args().any(|arg| arg == "--purge") => {
            if let Err(error) = management_agent::purge(&paths) {
                if error.starts_with("purge pending; helper transaction ") {
                    println!("{error}");
                    std::process::exit(10);
                }
                eprintln!("management agent purge failed: {error}");
                std::process::exit(1);
            }
        }
        "--cleanup" => {
            let root = arguments
                .get(1)
                .map(std::path::PathBuf::from)
                .unwrap_or_else(|| paths.root.clone());
            if let Err(error) =
                management_agent::cleanup(&root, config.normal_install_path.as_deref())
            {
                eprintln!("management agent cleanup failed: {error}");
                std::process::exit(1);
            }
        }
        "--service" => {
            if let Err(error) = management_agent::run_service(config, paths) {
                eprintln!("management agent service failed: {error}");
                std::process::exit(1);
            }
        }
        _ => {
            eprintln!(
                "usage: rustdesk-management-agent [--service|status|check-now|uninstall --purge]"
            );
            std::process::exit(2);
        }
    }
}
