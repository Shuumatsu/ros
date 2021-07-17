use log::{Level, LevelFilter, Log, Metadata, Record, SetLoggerError};

pub struct ColorLogger;

impl Log for ColorLogger {
    fn enabled(&self, metadata: &Metadata) -> bool {
        true
    }

    fn log(&self, record: &Record) {
        if !self.enabled(record.metadata()) {
            return;
        }

        match record.metadata().level() {
            Level::Trace => println!("\x1b[90m{}\x1b[0m", record.args()),
            Level::Debug => println!("\x1b[32m{}\x1b[0m", record.args()),
            Level::Info => println!("\x1b[34m{}\x1b[0m", record.args()),
            Level::Warn => println!("\x1b[93m{}\x1b[0m", record.args()),
            Level::Error => println!("\x1b[31m{}\x1b[0m", record.args()),
        };
    }

    fn flush(&self) {}
}

static LOGGER: ColorLogger = ColorLogger;

pub fn init() {
    let level = match option_env!("LOG") {
        Some("error") => LevelFilter::Error,
        Some("warn") => LevelFilter::Warn,
        Some("info") => LevelFilter::Info,
        Some("debug") => LevelFilter::Debug,
        Some("trace") => LevelFilter::Trace,
        _ => LevelFilter::Off,
    };
    log::set_logger(&LOGGER)
        .map(|()| log::set_max_level(level))
        .unwrap();
}
