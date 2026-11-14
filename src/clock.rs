use std::sync::Once;
use std::time::{SystemTime, UNIX_EPOCH};

static TZ: Once = Once::new();

unsafe extern "C" {
    fn tzset();
}

fn init_tz() {
    TZ.call_once(|| unsafe { tzset() });
}

pub fn now() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LocalTime {
    pub year: i32,
    pub month: u32,
    pub day: u32,
    pub hour: u32,
    pub minute: u32,
    pub second: u32,
}

impl LocalTime {
    pub fn minutes_of_day(&self) -> u32 {
        self.hour * 60 + self.minute
    }

    pub fn hhmm(&self) -> String {
        format!("{:02}:{:02}", self.hour, self.minute)
    }

    pub fn day_path(&self) -> String {
        format!("{:04}/{:02}/{:02}", self.year, self.month, self.day)
    }

    pub fn month_path(&self) -> String {
        format!("{:04}/{:02}", self.year, self.month)
    }

    pub fn date_minute(&self) -> String {
        format!(
            "{:04}-{:02}-{:02} {:02}:{:02}",
            self.year, self.month, self.day, self.hour, self.minute
        )
    }
}

pub fn local(ts: f64) -> LocalTime {
    init_tz();
    let secs = ts.floor() as _;
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    let ok = unsafe { !libc::localtime_r(&secs, &mut tm).is_null() };
    if !ok {
        return LocalTime {
            year: 1970,
            month: 1,
            day: 1,
            hour: 0,
            minute: 0,
            second: 0,
        };
    }
    LocalTime {
        year: tm.tm_year + 1900,
        month: (tm.tm_mon + 1) as u32,
        day: tm.tm_mday as u32,
        hour: tm.tm_hour as u32,
        minute: tm.tm_min as u32,
        second: tm.tm_sec as u32,
    }
}

pub fn mktime(year: i32, month: u32, day: u32, hour: u32, minute: u32, second: u32) -> f64 {
    init_tz();
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    tm.tm_year = year - 1900;
    tm.tm_mon = month as i32 - 1;
    tm.tm_mday = day as i32;
    tm.tm_hour = hour as i32;
    tm.tm_min = minute as i32;
    tm.tm_sec = second as i32;
    tm.tm_isdst = -1;
    unsafe { libc::mktime(&mut tm) as f64 }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_time_roundtrips_through_mktime() {
        let ts = mktime(2026, 10, 12, 23, 30, 5);
        let t = local(ts);
        assert_eq!(
            (t.year, t.month, t.day, t.hour, t.minute, t.second),
            (2026, 10, 12, 23, 30, 5)
        );
        assert_eq!(t.hhmm(), "23:30");
        assert_eq!(t.day_path(), "2026/10/12");
        assert_eq!(t.month_path(), "2026/10");
        assert_eq!(t.date_minute(), "2026-10-12 23:30");
    }

    #[test]
    fn now_is_after_2025() {
        assert!(now() > 1_735_689_600.0);
    }
}
