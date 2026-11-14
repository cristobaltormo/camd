mod codec;
mod session;

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{Map, Value, json};
use tokio::sync::{Notify, mpsc};

use crate::api::pyjson;
use crate::config::Config;
use crate::error::Result;

use session::{Failure, Request, Session, Settings};

pub const DISCOVERY_PREFIX: &str = "homeassistant";
pub const KEEPALIVE_S: u64 = 30;
pub const RETRY_S: u64 = 5;
pub const STOP_WAIT_S: u64 = 2;
pub const QUEUE: usize = 64;

pub type PresenceCommand = Arc<dyn Fn(Option<bool>) + Send + Sync>;
pub type ModeCommand = Arc<dyn Fn(&str) -> Result<()> + Send + Sync>;
pub type PrivacyCommand = Arc<dyn Fn(bool) -> bool + Send + Sync>;

#[derive(Clone)]
pub struct Commands {
    pub presence: PresenceCommand,
    pub mode: ModeCommand,
    pub privacy: PrivacyCommand,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    Subscribe(String),
    Publish(String, String),
}

#[derive(Default)]
struct Shared {
    last: Mutex<HashMap<String, String>>,
    requests: Mutex<Option<mpsc::Sender<Request>>>,
    stopping: AtomicBool,
    wake: Notify,
    done: Mutex<Option<std::sync::mpsc::Receiver<()>>>,
}

pub struct Mqtt {
    pub topic: String,
    pub enabled: bool,
    cfg: Config,
    shared: Arc<Shared>,
}

fn node_id(topic: &str) -> String {
    topic
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

fn capitalize(text: &str) -> String {
    let mut chars = text.chars();
    match chars.next() {
        Some(first) => first
            .to_uppercase()
            .chain(chars.flat_map(char::to_lowercase))
            .collect(),
        None => String::new(),
    }
}

fn binary_sensor(
    topic: &str,
    state: &str,
    device_class: Option<&str>,
    on: &str,
    off: &str,
) -> Value {
    let mut entity =
        json!({"state_topic": format!("{topic}/{state}"), "payload_on": on, "payload_off": off});
    if let Some(class) = device_class {
        entity["device_class"] = json!(class);
    }
    entity
}

fn entities(t: &str) -> Vec<(&'static str, &'static str, Value)> {
    let mut object = binary_sensor(t, "object", Some("occupancy"), "ON", "OFF");
    object["name"] = json!("Object detected");
    vec![
        (
            "binary_sensor",
            "motion",
            binary_sensor(t, "motion", Some("motion"), "ON", "OFF"),
        ),
        ("binary_sensor", "object", object),
        (
            "binary_sensor",
            "camera",
            binary_sensor(t, "camera", Some("connectivity"), "online", "offline"),
        ),
        (
            "binary_sensor",
            "armed",
            binary_sensor(t, "armed", None, "ON", "OFF"),
        ),
        (
            "select",
            "mode",
            json!({"state_topic": format!("{t}/mode"), "command_topic": format!("{t}/mode/set"), "options": ["auto", "armed", "disarmed"]}),
        ),
        (
            "switch",
            "privacy",
            json!({"state_topic": format!("{t}/privacy"), "command_topic": format!("{t}/privacy/set")}),
        ),
    ]
}

pub fn discovery(cfg: &Config) -> Vec<(String, Value)> {
    let topic = &cfg.mqtt_topic;
    let node = node_id(topic);
    let device = json!({"identifiers": [node], "name": cfg.site_name, "manufacturer": "camd"});
    entities(topic)
        .into_iter()
        .map(|(kind, key, entity)| {
            let mut payload = Map::new();
            payload.insert("name".into(), json!(capitalize(key)));
            payload.insert("unique_id".into(), json!(format!("{node}_{key}")));
            payload.insert(
                "availability_topic".into(),
                json!(format!("{topic}/status")),
            );
            payload.insert("device".into(), device.clone());
            if let Value::Object(fields) = entity {
                for (k, v) in fields {
                    payload.insert(k, v);
                }
            }
            (
                format!("{DISCOVERY_PREFIX}/{kind}/{node}/{key}/config"),
                Value::Object(payload),
            )
        })
        .collect()
}

fn tls_config() -> std::result::Result<rustls::ClientConfig, rustls::Error> {
    let mut roots = rustls::RootCertStore::empty();
    for cert in rustls_native_certs::load_native_certs().certs {
        let _ = roots.add(cert);
    }
    if roots.is_empty() {
        roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    }
    Ok(
        rustls::ClientConfig::builder_with_provider(crate::notify::tls_provider())
            .with_safe_default_protocol_versions()?
            .with_root_certificates(roots)
            .with_no_client_auth(),
    )
}

impl Mqtt {
    pub fn new(cfg: &Config) -> Mqtt {
        Mqtt {
            topic: cfg.mqtt_topic.clone(),
            enabled: !cfg.mqtt_host.is_empty(),
            cfg: cfg.clone(),
            shared: Arc::new(Shared::default()),
        }
    }

    pub fn settings(&self) -> std::result::Result<Settings, String> {
        let port =
            u16::try_from(self.cfg.mqtt_port).map_err(|_| "invalid mqtt_port".to_string())?;
        let tls = if self.cfg.mqtt_tls {
            Some(Arc::new(tls_config().map_err(|e| e.to_string())?))
        } else {
            None
        };
        let user = &self.cfg.mqtt_user;
        let password = &self.cfg.mqtt_password;
        Ok(Settings {
            host: self.cfg.mqtt_host.clone(),
            port,
            keep_alive: Duration::from_secs(KEEPALIVE_S),
            connect: codec::Connect {
                client_id: format!("camd-{}", self.topic),
                keep_alive: KEEPALIVE_S as u16,
                username: (!user.is_empty()).then(|| user.clone()),
                password: (!user.is_empty() && !password.is_empty()).then(|| password.clone()),
                will: Some(codec::Will {
                    topic: format!("{}/status", self.topic),
                    payload: b"offline".to_vec(),
                    retain: true,
                }),
            },
            tls,
        })
    }

    pub fn connected_actions(&self) -> Vec<Action> {
        let mut actions: Vec<Action> = ["presence", "mode", "privacy"]
            .iter()
            .map(|s| Action::Subscribe(format!("{}/{s}/set", self.topic)))
            .collect();
        actions.push(Action::Publish(
            format!("{}/status", self.topic),
            "online".into(),
        ));
        if self.cfg.mqtt_discovery {
            for (topic, payload) in discovery(&self.cfg) {
                actions.push(Action::Publish(topic, pyjson::dumps(&payload)));
            }
        }
        actions
    }

    pub fn start(&self, commands: Commands) {
        if !self.enabled {
            return;
        }
        let settings = match self.settings() {
            Ok(settings) => settings,
            Err(e) => {
                crate::warning!("camd.mqtt", "mqtt unavailable: {}", e);
                return;
            }
        };
        let (requests_tx, requests) = mpsc::channel(QUEUE);
        *self
            .shared
            .requests
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = Some(requests_tx);
        let (done_tx, done_rx) = std::sync::mpsc::channel();
        *self.shared.done.lock().unwrap_or_else(|e| e.into_inner()) = Some(done_rx);
        let worker = Mqtt {
            topic: self.topic.clone(),
            enabled: true,
            cfg: self.cfg.clone(),
            shared: Arc::clone(&self.shared),
        };
        let spawned = std::thread::Builder::new()
            .name("mqtt".into())
            .spawn(move || {
                match tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                {
                    Ok(runtime) => runtime.block_on(worker.run(settings, requests, &commands)),
                    Err(e) => crate::warning!("camd.mqtt", "mqtt unavailable: {}", e),
                }
                let _ = done_tx.send(());
            });
        if let Err(e) = spawned {
            crate::warning!("camd.mqtt", "mqtt unavailable: {}", e);
        }
    }

    async fn run(
        &self,
        settings: Settings,
        mut requests: mpsc::Receiver<Request>,
        commands: &Commands,
    ) {
        let mut reported = false;
        loop {
            let failure = match Session::open(&settings).await {
                Ok(mut session) => {
                    reported = false;
                    self.shared
                        .last
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .clear();
                    match self.serve(&mut session, &mut requests, commands).await {
                        Ok(()) => {
                            session.disconnect().await;
                            return;
                        }
                        Err(failure) => failure,
                    }
                }
                Err(failure) => failure,
            };
            if self.shared.stopping.load(Ordering::Relaxed) {
                return;
            }
            match failure {
                Failure::Refused(code) => {
                    crate::warning!("camd.mqtt", "mqtt connect refused: {:?}", code);
                }
                other if !reported => {
                    crate::warning!("camd.mqtt", "mqtt unavailable: {}", other);
                    reported = true;
                }
                _ => {}
            }
            tokio::select! {
                () = tokio::time::sleep(Duration::from_secs(RETRY_S)) => {}
                () = self.shared.wake.notified() => return,
            }
        }
    }

    async fn serve(
        &self,
        session: &mut Session,
        requests: &mut mpsc::Receiver<Request>,
        commands: &Commands,
    ) -> std::result::Result<(), Failure> {
        for action in self.connected_actions() {
            match action {
                Action::Subscribe(topic) => session.subscribe(&topic).await?,
                Action::Publish(topic, payload) => {
                    session.publish(&topic, payload.as_bytes()).await?;
                }
            }
        }
        session
            .run(requests, &mut |topic, payload| {
                self.handle_command(topic, payload, commands);
            })
            .await
    }

    pub fn publish(&self, suffix: &str, value: &str) {
        if !self.enabled {
            return;
        }
        {
            let mut last = self.shared.last.lock().unwrap_or_else(|e| e.into_inner());
            if last.get(suffix).map(String::as_str) == Some(value) {
                return;
            }
            last.insert(suffix.to_string(), value.to_string());
        }
        if let Some(requests) = self
            .shared
            .requests
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
        {
            let _ = requests.try_send(Request::Publish(
                format!("{}/{suffix}", self.topic),
                value.as_bytes().to_vec(),
            ));
        }
    }

    pub fn last_published(&self, suffix: &str) -> Option<String> {
        self.shared
            .last
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(suffix)
            .cloned()
    }

    pub fn stop(&self) {
        if !self.enabled {
            return;
        }
        self.publish("status", "offline");
        self.shared.stopping.store(true, Ordering::Relaxed);
        if let Some(requests) = self
            .shared
            .requests
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take()
        {
            let _ = requests.try_send(Request::Disconnect);
        }
        self.shared.wake.notify_one();
        if let Some(done) = self
            .shared
            .done
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take()
        {
            let _ = done.recv_timeout(Duration::from_secs(STOP_WAIT_S));
        }
    }

    pub fn handle_command(&self, topic: &str, payload: &[u8], commands: &Commands) {
        let value = String::from_utf8_lossy(payload)
            .replace('\u{FFFD}', "")
            .trim()
            .to_lowercase();
        let prefix = format!("{}/", self.topic);
        let target = topic.strip_prefix(&prefix).unwrap_or(topic);
        let target = target.strip_suffix("/set").unwrap_or(target);
        match target {
            "presence" => (commands.presence)(match value.as_str() {
                "home" => Some(true),
                "away" => Some(false),
                _ => None,
            }),
            "mode" => {
                if let Err(e) = (commands.mode)(&value) {
                    crate::warning!("camd.mqtt", "mqtt command {} rejected: {}", target, e);
                }
            }
            "privacy" => {
                (commands.privacy)(matches!(value.as_str(), "on" | "true" | "1"));
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;

    fn recorder() -> (Commands, Arc<Mutex<Vec<String>>>) {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let (a, b, c) = (Arc::clone(&seen), Arc::clone(&seen), Arc::clone(&seen));
        let commands = Commands {
            presence: Arc::new(move |v| a.lock().unwrap().push(format!("presence={v:?}"))),
            mode: Arc::new(move |v| {
                if crate::arming::MODES.contains(&v) {
                    b.lock().unwrap().push(format!("mode={v}"));
                    Ok(())
                } else {
                    Err(crate::error::Error::invalid("invalid mode"))
                }
            }),
            privacy: Arc::new(move |v| {
                c.lock().unwrap().push(format!("privacy={v}"));
                true
            }),
        };
        (commands, seen)
    }

    fn config(topic: &str) -> Config {
        Config {
            mqtt_host: "broker".into(),
            mqtt_topic: topic.into(),
            site_name: "Room".into(),
            ..Config::default()
        }
    }

    #[test]
    fn discovery_announces_every_entity() {
        let mqtt = Mqtt::new(&config("cam/room"));
        let actions = mqtt.connected_actions();
        assert_eq!(
            actions[0],
            Action::Subscribe("cam/room/presence/set".into())
        );
        assert!(actions.contains(&Action::Publish("cam/room/status".into(), "online".into())));
        let configs: HashMap<String, Value> = actions
            .iter()
            .filter_map(|a| match a {
                Action::Publish(t, p) if t.starts_with("homeassistant/") => {
                    Some((t.clone(), serde_json::from_str(p).unwrap()))
                }
                _ => None,
            })
            .collect();
        assert_eq!(configs.len(), 6);
        let motion = &configs["homeassistant/binary_sensor/cam_room/motion/config"];
        assert_eq!(motion["state_topic"], "cam/room/motion");
        assert_eq!(motion["device_class"], "motion");
        assert_eq!(motion["name"], "Motion");
        assert_eq!(motion["unique_id"], "cam_room_motion");
        assert_eq!(motion["availability_topic"], "cam/room/status");
        assert_eq!(
            motion["device"],
            json!({"identifiers": ["cam_room"], "name": "Room", "manufacturer": "camd"})
        );
        assert_eq!(
            configs["homeassistant/binary_sensor/cam_room/object/config"]["name"],
            "Object detected"
        );
        assert_eq!(
            configs["homeassistant/switch/cam_room/privacy/config"]["command_topic"],
            "cam/room/privacy/set"
        );
        assert_eq!(
            configs["homeassistant/select/cam_room/mode/config"]["options"],
            json!(["auto", "armed", "disarmed"])
        );
        assert_eq!(
            configs["homeassistant/binary_sensor/cam_room/camera/config"]["payload_on"],
            "online"
        );
        assert!(
            configs["homeassistant/binary_sensor/cam_room/armed/config"]
                .get("device_class")
                .is_none()
        );
        let without = Mqtt::new(&Config {
            mqtt_discovery: false,
            ..config("cam")
        });
        assert_eq!(without.connected_actions().len(), 4);
    }

    #[test]
    fn settings_follow_the_config() {
        let plain = Mqtt::new(&config("cam")).settings().unwrap();
        assert_eq!(plain.port, 1883);
        assert!(plain.tls.is_none());
        assert_eq!(
            plain.connect,
            codec::Connect {
                client_id: "camd-cam".into(),
                keep_alive: 30,
                username: None,
                password: None,
                will: Some(codec::Will {
                    topic: "cam/status".into(),
                    payload: b"offline".to_vec(),
                    retain: true,
                }),
            }
        );
        let secure = Mqtt::new(&Config {
            mqtt_user: "u".into(),
            mqtt_password: "p".into(),
            mqtt_tls: true,
            mqtt_port: 8883,
            ..config("cam")
        })
        .settings()
        .unwrap();
        assert!(secure.tls.is_some());
        assert_eq!(secure.connect.username.as_deref(), Some("u"));
        assert_eq!(secure.connect.password.as_deref(), Some("p"));
        let bad_port = Mqtt::new(&Config {
            mqtt_port: 70_000,
            ..config("cam")
        });
        assert!(bad_port.settings().is_err());
    }

    #[test]
    fn without_a_broker_it_is_inert() {
        let (commands, _) = recorder();
        let mqtt = Mqtt::new(&Config::default());
        mqtt.publish("motion", "ON");
        mqtt.start(commands);
        mqtt.stop();
        assert!(!mqtt.enabled);
        assert_eq!(mqtt.last_published("motion"), None);
    }

    #[test]
    fn commands_reach_the_runtime() {
        let (commands, seen) = recorder();
        let mqtt = Mqtt::new(&config("cam"));
        mqtt.handle_command("cam/presence/set", b"home", &commands);
        mqtt.handle_command("cam/mode/set", b" ARMED\n", &commands);
        mqtt.handle_command("cam/privacy/set", b"ON", &commands);
        mqtt.handle_command("cam/privacy/set", b"off", &commands);
        mqtt.handle_command("cam/presence/set", b"away", &commands);
        mqtt.handle_command("cam/presence/set", b"auto", &commands);
        mqtt.handle_command("cam/mode/set", b"bogus", &commands);
        mqtt.handle_command("cam/other/set", b"x", &commands);
        mqtt.handle_command("cam/mode/set", &[0xff, b'a', b'u', b't', b'o'], &commands);
        assert_eq!(
            *seen.lock().unwrap(),
            [
                "presence=Some(true)",
                "mode=armed",
                "privacy=true",
                "privacy=false",
                "presence=Some(false)",
                "presence=None",
                "mode=auto"
            ]
        );
    }

    #[test]
    fn publish_skips_repeated_values() {
        let mqtt = Mqtt::new(&config("cam"));
        mqtt.publish("motion", "ON");
        mqtt.publish("motion", "ON");
        assert_eq!(mqtt.last_published("motion").as_deref(), Some("ON"));
        mqtt.publish("motion", "OFF");
        assert_eq!(mqtt.last_published("motion").as_deref(), Some("OFF"));
    }

    fn read_packet(stream: &mut std::net::TcpStream) -> Option<(u8, Vec<u8>)> {
        let mut first = [0u8; 1];
        stream.read_exact(&mut first).ok()?;
        let mut length = 0usize;
        let mut shift = 0;
        loop {
            let mut byte = [0u8; 1];
            stream.read_exact(&mut byte).ok()?;
            length |= usize::from(byte[0] & 0x7f) << shift;
            shift += 7;
            if byte[0] & 0x80 == 0 {
                break;
            }
        }
        let mut body = vec![0u8; length];
        stream.read_exact(&mut body).ok()?;
        Some((first[0], body))
    }

    fn mqtt_string(body: &[u8], at: usize) -> (String, usize) {
        let len = usize::from(u16::from_be_bytes([body[at], body[at + 1]]));
        (
            String::from_utf8_lossy(&body[at + 2..at + 2 + len]).into_owned(),
            at + 2 + len,
        )
    }

    fn publish_packet(topic: &str, payload: &[u8]) -> Vec<u8> {
        let mut body = (topic.len() as u16).to_be_bytes().to_vec();
        body.extend_from_slice(topic.as_bytes());
        body.extend_from_slice(payload);
        let mut packet = vec![0x30, body.len() as u8];
        packet.extend_from_slice(&body);
        packet
    }

    #[test]
    fn talks_to_a_real_broker_socket() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut subscribed = 0;
            while let Some((kind, body)) = read_packet(&mut stream) {
                match kind >> 4 {
                    1 => {
                        let (protocol, at) = mqtt_string(&body, 0);
                        let flags = body[at + 1];
                        let (client_id, at) = mqtt_string(&body, at + 4);
                        let (will_topic, at) = mqtt_string(&body, at);
                        let (will, _) = mqtt_string(&body, at);
                        tx.send(format!(
                            "connect {protocol} {client_id} will={will_topic}:{will} retain={}",
                            flags & 0x20 != 0
                        ))
                        .unwrap();
                        stream.write_all(&[0x20, 2, 0, 0]).unwrap();
                    }
                    3 => {
                        let (topic, at) = mqtt_string(&body, 0);
                        let payload = String::from_utf8_lossy(&body[at..]).into_owned();
                        if !topic.starts_with("homeassistant/") {
                            tx.send(format!(
                                "publish {topic} {payload} retain={}",
                                kind & 1 == 1
                            ))
                            .unwrap();
                        }
                    }
                    8 => {
                        let (topic, _) = mqtt_string(&body, 2);
                        tx.send(format!("subscribe {topic}")).unwrap();
                        stream.write_all(&[0x90, 3, body[0], body[1], 0]).unwrap();
                        subscribed += 1;
                        if subscribed == 3 {
                            stream
                                .write_all(&publish_packet("cam/mode/set", b"armed"))
                                .unwrap();
                        }
                    }
                    14 => {
                        tx.send("disconnect".into()).unwrap();
                        return;
                    }
                    _ => {}
                }
            }
        });
        let (commands, seen) = recorder();
        let mqtt = Mqtt::new(&Config {
            mqtt_port: i64::from(port),
            mqtt_host: "127.0.0.1".into(),
            ..config("cam")
        });
        mqtt.start(commands);
        let mut log = Vec::new();
        while log.len() < 5 {
            log.push(rx.recv_timeout(Duration::from_secs(5)).unwrap());
        }
        assert_eq!(
            log[0],
            "connect MQTT camd-cam will=cam/status:offline retain=true"
        );
        assert!(log.contains(&"subscribe cam/mode/set".to_string()));
        assert!(log.contains(&"publish cam/status online retain=true".to_string()));
        for _ in 0..50 {
            if !seen.lock().unwrap().is_empty() {
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        assert_eq!(*seen.lock().unwrap(), ["mode=armed"]);
        mqtt.publish("motion", "ON");
        mqtt.publish("motion", "ON");
        assert_eq!(
            rx.recv_timeout(Duration::from_secs(5)).unwrap(),
            "publish cam/motion ON retain=true"
        );
        mqtt.stop();
        assert_eq!(
            rx.recv_timeout(Duration::from_secs(5)).unwrap(),
            "publish cam/status offline retain=true"
        );
        assert_eq!(
            rx.recv_timeout(Duration::from_secs(5)).unwrap(),
            "disconnect"
        );
    }

    fn mosquitto() -> Option<String> {
        let candidates = [
            std::env::var("CAMD_MOSQUITTO").unwrap_or_default(),
            "/usr/sbin/mosquitto".to_string(),
        ];
        candidates
            .into_iter()
            .find(|path| !path.is_empty() && std::path::Path::new(path).is_file())
    }

    struct Broker(std::process::Child);

    impl Broker {
        fn start(binary: &str, dir: &std::path::Path, port: u16) -> Broker {
            let conf = dir.join("mosquitto.conf");
            std::fs::write(
                &conf,
                format!("listener {port} 127.0.0.1\nallow_anonymous true\npersistence false\n"),
            )
            .unwrap();
            let child = std::process::Command::new(binary)
                .arg("-c")
                .arg(&conf)
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn()
                .unwrap();
            for _ in 0..100 {
                if std::net::TcpStream::connect(("127.0.0.1", port)).is_ok() {
                    break;
                }
                std::thread::sleep(Duration::from_millis(50));
            }
            Broker(child)
        }
    }

    impl Drop for Broker {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }

    struct Observer {
        stream: std::net::TcpStream,
        buffer: Vec<u8>,
    }

    impl Observer {
        fn connect(port: u16, filter: &str) -> Observer {
            let mut stream = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(10)))
                .unwrap();
            let hello = codec::Connect {
                client_id: "observer".into(),
                keep_alive: 60,
                username: None,
                password: None,
                will: None,
            };
            stream.write_all(&codec::connect(&hello).unwrap()).unwrap();
            stream
                .write_all(&codec::subscribe(1, filter).unwrap())
                .unwrap();
            Observer {
                stream,
                buffer: Vec::new(),
            }
        }

        fn next(&mut self) -> codec::Packet {
            loop {
                if let Some((packet, used)) = codec::decode(&self.buffer).unwrap() {
                    self.buffer.drain(..used);
                    return packet;
                }
                let mut chunk = [0u8; 4096];
                let read = self.stream.read(&mut chunk).unwrap();
                assert!(read > 0, "broker closed the observer");
                self.buffer.extend_from_slice(&chunk[..read]);
            }
        }

        fn wait_for(&mut self, topic: &str, payload: &str) -> bool {
            loop {
                if let codec::Packet::Publish {
                    topic: t,
                    payload: p,
                    retain,
                    ..
                } = self.next()
                    && t == topic
                    && p == payload.as_bytes()
                {
                    return retain;
                }
            }
        }

        fn send(&mut self, topic: &str, payload: &str) {
            self.stream
                .write_all(&codec::publish(topic, payload.as_bytes(), false).unwrap())
                .unwrap();
        }
    }

    #[test]
    fn works_with_mosquitto() {
        let Some(binary) = mosquitto() else {
            eprintln!("mosquitto not found, set CAMD_MOSQUITTO to run this test");
            return;
        };
        let dir = tempfile::tempdir().unwrap();
        let port = TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let broker = Broker::start(&binary, dir.path(), port);
        let mut observer = Observer::connect(port, "cam/#");
        let (commands, seen) = recorder();
        let mqtt = Mqtt::new(&Config {
            mqtt_port: i64::from(port),
            mqtt_host: "127.0.0.1".into(),
            ..config("cam")
        });
        mqtt.start(commands);
        observer.wait_for("cam/status", "online");
        observer.send("cam/privacy/set", "ON");
        for _ in 0..100 {
            if !seen.lock().unwrap().is_empty() {
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        assert_eq!(*seen.lock().unwrap(), ["privacy=true"]);
        mqtt.publish("motion", "ON");
        observer.wait_for("cam/motion", "ON");
        let mut late = Observer::connect(port, "cam/motion");
        assert!(late.wait_for("cam/motion", "ON"), "state is retained");
        drop(late);

        drop(observer);
        drop(broker);
        let broker = Broker::start(&binary, dir.path(), port);
        let mut observer = Observer::connect(port, "cam/status");
        observer.wait_for("cam/status", "online");
        mqtt.stop();
        observer.wait_for("cam/status", "offline");
        drop(broker);
    }
}
