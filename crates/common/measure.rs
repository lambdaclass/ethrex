use std::fmt::Display;
use std::time::Instant;
use tracing::info;

pub fn start_timer() -> Instant {
    Instant::now()
}

pub fn stop_timer(start: Instant, message: impl Display) {
    let elapsed = start.elapsed();
    info!("[TIME MEASURE]: {} took {:?}", message, elapsed);
}
