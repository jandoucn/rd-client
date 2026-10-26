use hbb_common::log;
use std::{any::Any, panic::Location};

// Release builds abort on panic and GUI/detached processes have no console,
// so the default hook's stderr output is lost; the file logger writes
// synchronously and still records the line before the abort.
pub fn install() {
    install_with(|line| log::error!("{}", line));
}

fn install_with(record: fn(&str)) {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        record(&describe(info.payload(), info.location()));
        previous(info);
    }));
}

fn describe(payload: &(dyn Any + Send), location: Option<&Location<'_>>) -> String {
    let message = payload
        .downcast_ref::<&str>()
        .copied()
        .or_else(|| payload.downcast_ref::<String>().map(String::as_str))
        .unwrap_or("non-string payload");
    let location = location
        .map(|l| format!("{}:{}:{}", l.file(), l.line(), l.column()))
        .unwrap_or_else(|| "unknown location".to_owned());
    format!("panic at {}: {}", location, message)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn describe_reports_str_payload_and_location() {
        let location = Location::caller();
        let text = describe(&"boom", Some(location));
        assert!(text.contains("boom"), "{text}");
        assert!(
            text.contains(&format!("{}:{}:", location.file(), location.line())),
            "{text}"
        );
    }

    #[test]
    fn describe_reports_string_payload() {
        let payload: Box<dyn Any + Send> = Box::new(String::from("index out of range"));
        assert!(describe(payload.as_ref(), None).contains("index out of range"));
    }

    #[test]
    fn describe_marks_unknown_payload_and_location() {
        let text = describe(&42u32, None);
        assert!(text.contains("non-string payload"), "{text}");
        assert!(text.contains("unknown location"), "{text}");
    }

    #[test]
    fn hook_records_panic_and_keeps_previous_hook() {
        use std::sync::{
            atomic::{AtomicUsize, Ordering},
            Mutex,
        };
        static RECORDED: Mutex<Vec<String>> = Mutex::new(Vec::new());
        static PREVIOUS: AtomicUsize = AtomicUsize::new(0);
        let default = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            if info.payload().downcast_ref::<&str>() == Some(&"panic_log_probe") {
                PREVIOUS.fetch_add(1, Ordering::SeqCst);
            } else {
                default(info);
            }
        }));
        install_with(|line| {
            if line.contains("panic_log_probe") {
                RECORDED.lock().unwrap().push(line.to_owned());
            }
        });
        let result = std::panic::catch_unwind(|| panic!("panic_log_probe"));
        let _ = std::panic::take_hook();
        assert!(result.is_err());
        let recorded = RECORDED.lock().unwrap();
        assert_eq!(recorded.len(), 1, "{recorded:?}");
        assert!(recorded[0].contains("panic_log_probe"), "{recorded:?}");
        assert!(recorded[0].contains("panic_log.rs"), "{recorded:?}");
        assert_eq!(PREVIOUS.load(Ordering::SeqCst), 1);
    }
}
