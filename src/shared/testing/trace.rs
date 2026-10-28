use std::io::{self, Write};
use std::sync::{Arc, Mutex};

struct TraceBufferWriter(Arc<Mutex<Vec<u8>>>);

impl Write for TraceBufferWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let mut output = self
            .0
            .lock()
            .map_err(|_| io::Error::other("trace buffer lock was poisoned"))?;
        output.extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

pub(crate) fn capture_json_events<T>(
    level: tracing::Level,
    operation: impl FnOnce() -> T,
) -> (T, Vec<serde_json::Value>) {
    use tracing_subscriber::prelude::*;

    let log_output = Arc::new(Mutex::new(Vec::new()));
    let writer_output = log_output.clone();
    let fmt_layer = tracing_subscriber::fmt::layer()
        .json()
        .flatten_event(true)
        .without_time()
        .with_level(false)
        .with_target(false)
        .with_writer(move || TraceBufferWriter(writer_output.clone()))
        .with_filter(tracing_subscriber::filter::filter_fn(move |metadata| {
            *metadata.level() == level
        }));
    let subscriber = tracing_subscriber::registry().with(fmt_layer);
    let result = tracing::subscriber::with_default(subscriber, operation);
    let log_output = log_output.lock().expect("trace buffer lock").clone();
    let log_events = String::from_utf8(log_output)
        .expect("trace output should be UTF-8")
        .lines()
        .map(serde_json::from_str)
        .collect::<Result<Vec<serde_json::Value>, _>>()
        .expect("trace output should contain JSON events");

    (result, log_events)
}
