pub fn disabled_exit_code(args: &[String]) -> Option<i32> {
    (args.first().map(String::as_str) == Some("--update")
        && args.iter().any(|arg| {
            arg == "--management-agent-outer-package"
                || arg.starts_with("--management-agent-outer-package=")
        }))
    .then_some(1)
}
