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
    let arguments = std::env::args_os().skip(1).collect::<Vec<_>>();
    let command = arguments
        .first()
        .map(|argument| argument.to_string_lossy().into_owned())
        .unwrap_or_else(|| "--service".to_owned());
    if command == "--management-agent-recovery-helper" {
        if let Err(error) = management_agent::run_recovery_helper(&arguments) {
            eprintln!("management agent recovery helper failed: {error}");
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
        "uninstall" if std::env::args().any(|arg| arg == "--purge") => {
            if let Err(error) = management_agent::purge(&paths) {
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
