//! Packaged builds have no terminal, so stderr-only panics vanish.
//! This routes them through the file logger first.

use std::backtrace::Backtrace;
use std::panic::PanicHookInfo;

pub fn install() {
    let prev = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let location = info.location().map_or_else(
            || "<unknown>".to_string(),
            |l| format!("{}:{}:{}", l.file(), l.line(), l.column()),
        );
        let backtrace = Backtrace::force_capture();
        log::error!(
            "panic at {location}: {payload}\n{backtrace}",
            payload = payload_string(info)
        );
        prev(info);
    }));
}

fn payload_string(info: &PanicHookInfo<'_>) -> String {
    if let Some(s) = info.payload().downcast_ref::<&'static str>() {
        (*s).to_string()
    } else if let Some(s) = info.payload().downcast_ref::<String>() {
        s.clone()
    } else {
        "<non-string panic>".to_string()
    }
}
