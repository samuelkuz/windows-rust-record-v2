use std::{
    fmt,
    fs::{self, File, OpenOptions},
    io::Write,
    path::PathBuf,
    sync::{
        Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::{SystemTime, UNIX_EPOCH},
};

use tracing::{
    Event, Level, Metadata, Subscriber,
    field::{Field, Visit},
    level_filters::LevelFilter,
    span::{Attributes, Id, Record},
    subscriber::Interest,
};

use crate::{AppResult, config::RecorderConfig};

pub(crate) fn init(config: &RecorderConfig) -> AppResult<PathBuf> {
    let log_dir = config.output_dir.join("logs");
    fs::create_dir_all(&log_dir)?;
    let log_path = log_dir.join("app.log");
    let file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)?;

    let subscriber = FileSubscriber {
        file: Mutex::new(file),
        next_span_id: AtomicU64::new(1),
    };

    tracing::subscriber::set_global_default(subscriber)
        .map_err(|error| format!("Could not initialize diagnostics logging: {error}"))?;

    tracing::info!(
        log_path = %log_path.display(),
        "diagnostics logging initialized"
    );
    Ok(log_path)
}

struct FileSubscriber {
    file: Mutex<File>,
    next_span_id: AtomicU64,
}

impl Subscriber for FileSubscriber {
    fn enabled(&self, _metadata: &Metadata<'_>) -> bool {
        true
    }

    fn register_callsite(&self, _metadata: &'static Metadata<'static>) -> Interest {
        Interest::always()
    }

    fn max_level_hint(&self) -> Option<LevelFilter> {
        Some(LevelFilter::TRACE)
    }

    fn new_span(&self, _span: &Attributes<'_>) -> Id {
        Id::from_u64(self.next_span_id.fetch_add(1, Ordering::Relaxed))
    }

    fn record(&self, _span: &Id, _values: &Record<'_>) {}

    fn record_follows_from(&self, _span: &Id, _follows: &Id) {}

    fn event(&self, event: &Event<'_>) {
        let metadata = event.metadata();
        let mut visitor = FieldVisitor::default();
        event.record(&mut visitor);

        let line = format_log_line(metadata, &visitor);
        if let Ok(mut file) = self.file.lock() {
            let _ = writeln!(file, "{line}");
            let _ = file.flush();
        }
    }

    fn enter(&self, _span: &Id) {}

    fn exit(&self, _span: &Id) {}

    fn clone_span(&self, id: &Id) -> Id {
        Id::from_u64(id.into_u64())
    }

    fn try_close(&self, _id: Id) -> bool {
        true
    }
}

#[derive(Default)]
struct FieldVisitor {
    message: Option<String>,
    fields: Vec<String>,
}

impl Visit for FieldVisitor {
    fn record_str(&mut self, field: &Field, value: &str) {
        self.record_display(field, value);
    }

    fn record_debug(&mut self, field: &Field, value: &dyn fmt::Debug) {
        if field.name() == "message" {
            self.message = Some(format!("{value:?}"));
        } else {
            self.fields.push(format!("{}={value:?}", field.name()));
        }
    }
}

impl FieldVisitor {
    fn record_display(&mut self, field: &Field, value: impl fmt::Display) {
        if field.name() == "message" {
            self.message = Some(value.to_string());
        } else {
            self.fields.push(format!("{}={value}", field.name()));
        }
    }
}

fn format_log_line(metadata: &Metadata<'_>, visitor: &FieldVisitor) -> String {
    let message = visitor.message.as_deref().unwrap_or("");
    let fields = if visitor.fields.is_empty() {
        String::new()
    } else {
        format!(" {}", visitor.fields.join(" "))
    };

    format!(
        "{} {:<5} {}:{} {}{}",
        timestamp(),
        level_label(metadata.level()),
        metadata.target(),
        metadata.line().unwrap_or(0),
        message,
        fields
    )
}

fn timestamp() -> String {
    match SystemTime::now().duration_since(UNIX_EPOCH) {
        Ok(duration) => format!("{}.{:03}", duration.as_secs(), duration.subsec_millis()),
        Err(_) => "0.000".to_string(),
    }
}

fn level_label(level: &Level) -> &'static str {
    match *level {
        Level::ERROR => "ERROR",
        Level::WARN => "WARN",
        Level::INFO => "INFO",
        Level::DEBUG => "DEBUG",
        Level::TRACE => "TRACE",
    }
}
