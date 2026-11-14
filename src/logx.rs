use std::fmt;
use std::io::Write;
use std::sync::atomic::{AtomicU8, Ordering};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
#[repr(u8)]
pub enum Level {
    Debug = 0,
    Info = 1,
    Warning = 2,
    Error = 3,
    Off = 4,
}

impl Level {
    pub fn name(self) -> &'static str {
        match self {
            Level::Debug => "DEBUG",
            Level::Info => "INFO",
            Level::Warning => "WARNING",
            Level::Error => "ERROR",
            Level::Off => "OFF",
        }
    }
}

static LEVEL: AtomicU8 = AtomicU8::new(Level::Info as u8);

pub fn set_level(level: Level) {
    LEVEL.store(level as u8, Ordering::Relaxed);
}

pub fn enabled(level: Level) -> bool {
    level as u8 >= LEVEL.load(Ordering::Relaxed)
}

pub fn line(level: Level, target: &str, args: fmt::Arguments<'_>) -> String {
    format!("{} {}: {}", level.name(), target, args)
}

pub fn emit(level: Level, target: &str, args: fmt::Arguments<'_>) {
    if !enabled(level) {
        return;
    }
    let mut text = line(level, target, args);
    text.push('\n');
    let _ = std::io::stderr().lock().write_all(text.as_bytes());
}

#[macro_export]
macro_rules! debug {
    ($target:expr, $($arg:tt)*) => {
        $crate::logx::emit($crate::logx::Level::Debug, $target, format_args!($($arg)*))
    };
}

#[macro_export]
macro_rules! info {
    ($target:expr, $($arg:tt)*) => {
        $crate::logx::emit($crate::logx::Level::Info, $target, format_args!($($arg)*))
    };
}

#[macro_export]
macro_rules! warning {
    ($target:expr, $($arg:tt)*) => {
        $crate::logx::emit($crate::logx::Level::Warning, $target, format_args!($($arg)*))
    };
}

#[macro_export]
macro_rules! error {
    ($target:expr, $($arg:tt)*) => {
        $crate::logx::emit($crate::logx::Level::Error, $target, format_args!($($arg)*))
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lines_follow_the_python_format() {
        assert_eq!(
            line(
                Level::Warning,
                "camd.storage",
                format_args!("could not delete {}", "/x")
            ),
            "WARNING camd.storage: could not delete /x"
        );
    }
}
