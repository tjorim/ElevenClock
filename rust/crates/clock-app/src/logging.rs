//! Application-side logging set-up (R11). The bounded writer, redaction and the
//! transition lines themselves live in `clock-windows`, next to the runtime that
//! produces them; this only turns logging on and records the outer lifecycle.
//!
//! Logging is best effort: it can never stop the clock, and nothing leaves the PC.
use clock_windows::logging;

/// Start the bounded local log and make an unexpected panic leave a line in it.
/// Call once, by the primary instance only.
pub fn start() {
    logging::init();
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let message = info
            .payload()
            .downcast_ref::<&str>()
            .map(|text| (*text).to_owned())
            .or_else(|| info.payload().downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "no message".to_owned());
        logging::log(&format!("internal error: {message}"));
        previous(info);
    }));
    logging::log(&format!(
        "ElevenClock-rs {} started",
        env!("CARGO_PKG_VERSION")
    ));
}

/// Record a failure that is about to be shown to the user.
pub fn failed(error: &dyn std::fmt::Display) {
    logging::log(&format!("stopped: {error}"));
}

pub fn stopped() {
    logging::log("ElevenClock-rs stopped");
}
