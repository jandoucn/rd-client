pub fn choice(options: &str) -> Result<&'static str, &'static str> {
    let mut selected = "inherit";
    for token in options.split_whitespace() {
        if let Some(value) = token.strip_prefix("defender=") {
            let value = match value {
                "1" => "1",
                "0" => "0",
                _ => return Err("Defender option must be 0 or 1"),
            };
            if selected != "inherit" && selected != value {
                return Err("conflicting Defender options");
            }
            selected = value;
        }
    }
    Ok(selected)
}

pub fn silent_option<'a>(
    arguments: impl IntoIterator<Item = &'a str>,
) -> Result<&'static str, &'static str> {
    let options = arguments.into_iter().collect::<Vec<_>>().join(" ");
    match choice(&options)? {
        "1" => Ok(" defender=1"),
        "0" => Ok(" defender=0"),
        _ => Ok(""),
    }
}
