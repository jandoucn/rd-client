#[cfg(all(windows, not(test)))]
#[allow(dead_code)]
#[path = "../src/management_agent.rs"]
mod management_agent;

fn main() {
    #[cfg(all(windows, not(test)))]
    match management_agent::inspect_purge_identity() {
        Ok(observation) => println!("READ_ONLY_VERIFIED={observation}"),
        Err(error) => {
            eprintln!("READ_ONLY_REFUSED={error}");
            std::process::exit(1);
        }
    }
}
