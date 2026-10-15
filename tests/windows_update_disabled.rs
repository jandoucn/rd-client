#[path = "../src/platform/windows_update_gate.rs"]
mod gate;
#[path = "../libs/portable/src/update_args.rs"]
mod update_args;

#[test]
fn runner_child() {
    let Ok(case) = std::env::var("ONLINE_DISABLED_FIXTURE") else { return };
    let mut args = vec![if case == "legacy" { "--update" } else { "--management-agent-exe-update" }.to_owned()];
    if case == "event" { args.extend(["--update-event".into(), "invalid-event".into()]); }
    let args = update_args::forwarded(&args, std::path::Path::new(r"C:\unused.exe"))
        .into_iter().map(|v| v.into_string().unwrap()).collect::<Vec<_>>();
    if let Some(code) = gate::disabled_exit_code(&args) { std::process::exit(code); }
    // Models core None -> Flutter runner EXIT_SUCCESS; no installation is run.
    std::process::exit(0);
}

#[test]
fn disabled_explicit_runner_exit_is_not_success() {
    for case in ["explicit", "event", "legacy"] {
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "runner_child", "--nocapture"])
            .env("ONLINE_DISABLED_FIXTURE", case).status().unwrap();
        assert_eq!(status.code(), Some(if case == "legacy" { 0 } else { 1 }), "{case}");
    }
}

#[test]
fn unrelated_arguments_do_not_gain_an_exit_gate() {
    for args in [vec![], vec!["--update"], vec!["--connect", "--management-agent-outer-package"],
        vec!["--update", "--management-agent-outer-package-extra"]] {
        assert_eq!(gate::disabled_exit_code(&args.into_iter().map(str::to_owned).collect::<Vec<_>>()), None);
    }
}
