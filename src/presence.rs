use std::process::{Command, Stdio};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use crate::clock;

pub const PROBE_TIMEOUT_S: f64 = 5.0;
pub const INTERVAL_S: f64 = 30.0;

fn on_path(program: &str) -> bool {
    std::env::var_os("PATH")
        .map(|paths| std::env::split_paths(&paths).any(|dir| dir.join(program).is_file()))
        .unwrap_or(false)
}

fn run_probe(args: &[&str]) -> bool {
    let Ok(mut child) = Command::new(args[0])
        .args(&args[1..])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    else {
        return false;
    };
    let deadline = Instant::now() + Duration::from_secs_f64(PROBE_TIMEOUT_S);
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return status.success(),
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(50)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return false;
            }
        }
    }
}

pub fn responds(ip: &str) -> bool {
    if run_probe(&["ping", "-c", "1", "-W", "1", ip]) {
        return true;
    }
    on_path("arping") && run_probe(&["arping", "-c", "2", "-w", "2", ip])
}

struct State {
    grace_s: f64,
    external: Option<bool>,
    last_seen: f64,
    halted: bool,
}

pub struct Presence {
    pub devices: Vec<String>,
    pub interval: Duration,
    state: Mutex<State>,
    wake: Condvar,
}

impl Presence {
    pub fn new(devices: Vec<String>, grace_s: f64) -> Arc<Presence> {
        Presence::with_interval(devices, grace_s, Duration::from_secs_f64(INTERVAL_S))
    }

    pub fn with_interval(devices: Vec<String>, grace_s: f64, interval: Duration) -> Arc<Presence> {
        Arc::new(Presence {
            devices,
            interval,
            state: Mutex::new(State {
                grace_s,
                external: None,
                last_seen: clock::now(),
                halted: false,
            }),
            wake: Condvar::new(),
        })
    }

    fn state(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn configured(&self) -> bool {
        !self.devices.is_empty() || self.state().external.is_some()
    }

    pub fn is_home(&self) -> bool {
        let state = self.state();
        match state.external {
            Some(home) => home,
            None => clock::now() - state.last_seen < state.grace_s,
        }
    }

    pub fn external(&self) -> Option<bool> {
        self.state().external
    }

    pub fn set_external(&self, home: Option<bool>) {
        self.state().external = home;
        if home == Some(true) {
            self.mark_seen();
        }
    }

    pub fn grace_s(&self) -> f64 {
        self.state().grace_s
    }

    pub fn set_grace_s(&self, grace_s: f64) {
        self.state().grace_s = grace_s;
    }

    pub fn mark_seen(&self) {
        self.state().last_seen = clock::now();
    }

    pub fn last_seen(&self) -> f64 {
        self.state().last_seen
    }

    pub fn set_last_seen(&self, ts: f64) {
        self.state().last_seen = ts;
    }

    pub fn stop(&self) {
        self.state().halted = true;
        self.wake.notify_all();
    }

    pub fn is_halted(&self) -> bool {
        self.state().halted
    }

    pub fn start(self: &Arc<Self>) -> std::io::Result<JoinHandle<()>> {
        match crate::sniff::start(self) {
            Ok(true) => crate::info!(
                "camd.presence",
                "listening passively for {:?}",
                self.devices
            ),
            Ok(false) => {}
            Err(e) => crate::warning!(
                "camd.presence",
                "passive listening unavailable, probing only: {e}"
            ),
        }
        let me = Arc::clone(self);
        std::thread::Builder::new()
            .name("presence".into())
            .spawn(move || me.run(responds))
    }

    pub fn run(&self, probe: fn(&str) -> bool) {
        loop {
            if self.state().halted {
                return;
            }
            if self.devices.iter().any(|ip| probe(ip)) {
                self.mark_seen();
            }
            let state = self.state();
            if state.halted {
                return;
            }
            let (state, _) = self
                .wake
                .wait_timeout(state, self.interval)
                .unwrap_or_else(|e| e.into_inner());
            if state.halted {
                return;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn without_devices_it_is_not_configured_and_counts_as_home_for_the_grace() {
        let presence = Presence::new(vec![], 300.0);
        assert!(!presence.configured());
        assert!(presence.is_home());
        presence.set_last_seen(clock::now() - 301.0);
        assert!(!presence.is_home());
    }

    #[test]
    fn external_presence_wins_and_marks_seen() {
        let presence = Presence::new(vec![], 300.0);
        presence.set_last_seen(0.0);
        presence.set_external(Some(false));
        assert!(presence.configured() && !presence.is_home());
        presence.set_external(Some(true));
        assert!(presence.is_home() && presence.last_seen() > 1.0);
        presence.set_last_seen(0.0);
        presence.set_external(None);
        assert!(!presence.configured() && !presence.is_home());
    }

    #[test]
    fn a_device_that_answers_keeps_someone_home() {
        let presence =
            Presence::with_interval(vec!["10.0.0.1".into()], 30.0, Duration::from_millis(20));
        presence.set_last_seen(0.0);
        assert!(presence.configured() && !presence.is_home());
        let runner = Arc::clone(&presence);
        let handle = std::thread::spawn(move || runner.run(|ip| ip == "10.0.0.1"));
        std::thread::sleep(Duration::from_millis(60));
        presence.stop();
        handle.join().unwrap();
        assert!(presence.is_home());
    }

    #[test]
    fn grace_can_change_live() {
        let presence = Presence::new(vec![], 300.0);
        presence.set_last_seen(clock::now() - 100.0);
        presence.set_grace_s(60.0);
        assert!(!presence.is_home());
        assert_eq!(presence.grace_s(), 60.0);
    }

    #[test]
    fn an_unreachable_address_does_not_respond() {
        assert!(!run_probe(&["/nonexistent/ping"]));
        assert!(!run_probe(&["false"]));
        assert!(run_probe(&["true"]));
    }
}
