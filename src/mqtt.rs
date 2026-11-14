use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use rumqttc::{
    Client, ConnectReturnCode, ConnectionError, Event, LastWill, MqttOptions, Outgoing, Packet,
    QoS, TlsConfiguration, Transport,
};
use serde_json::{Map, Value, json};

use crate::api::pyjson;
use crate::config::Config;
use crate::error::Result;

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
    client: Mutex<Option<Client>>,
    stopping: AtomicBool,
    done: Mutex<Option<mpsc::Receiver<()>>>,
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

    pub fn options(&self) -> std::result::Result<MqttOptions, String> {
        let port =
            u16::try_from(self.cfg.mqtt_port).map_err(|_| "invalid mqtt_port".to_string())?;
        let mut options =
            MqttOptions::new(format!("camd-{}", self.topic), &self.cfg.mqtt_host, port);
        options.set_keep_alive(Duration::from_secs(KEEPALIVE_S));
        if !self.cfg.mqtt_user.is_empty() {
            options.set_credentials(&self.cfg.mqtt_user, &self.cfg.mqtt_password);
        }
        options.set_last_will(LastWill::new(
            format!("{}/status", self.topic),
            "offline",
            QoS::AtMostOnce,
            true,
        ));
        if self.cfg.mqtt_tls {
            let tls = tls_config().map_err(|e| e.to_string())?;
            options.set_transport(Transport::tls_with_config(TlsConfiguration::Rustls(
                Arc::new(tls),
            )));
        }
        Ok(options)
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
        let options = match self.options() {
            Ok(options) => options,
            Err(e) => {
                crate::warning!("camd.mqtt", "mqtt unavailable: {}", e);
                return;
            }
        };
        let (client, connection) = Client::new(options, QUEUE);
        *self.shared.client.lock().unwrap_or_else(|e| e.into_inner()) = Some(client.clone());
        let (done_tx, done_rx) = mpsc::channel();
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
                worker.run(client, connection, &commands);
                let _ = done_tx.send(());
            });
        if let Err(e) = spawned {
            crate::warning!("camd.mqtt", "mqtt unavailable: {}", e);
        }
    }

    fn pause(&self) {
        for _ in 0..RETRY_S * 10 {
            if self.shared.stopping.load(Ordering::Relaxed) {
                return;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    }

    fn run(&self, client: Client, mut connection: rumqttc::Connection, commands: &Commands) {
        let mut reported = false;
        for notification in connection.iter() {
            let stopping = self.shared.stopping.load(Ordering::Relaxed);
            match notification {
                Ok(Event::Incoming(Packet::ConnAck(ack))) => {
                    if ack.code != ConnectReturnCode::Success {
                        crate::warning!("camd.mqtt", "mqtt connect refused: {:?}", ack.code);
                        continue;
                    }
                    reported = false;
                    self.shared
                        .last
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .clear();
                    for action in self.connected_actions() {
                        let _ = match action {
                            Action::Subscribe(topic) => {
                                client.try_subscribe(topic, QoS::AtMostOnce)
                            }
                            Action::Publish(topic, payload) => {
                                client.try_publish(topic, QoS::AtMostOnce, true, payload)
                            }
                        };
                    }
                }
                Ok(Event::Incoming(Packet::Publish(message))) => {
                    self.handle_command(&message.topic, &message.payload, commands);
                }
                Ok(Event::Outgoing(Outgoing::Disconnect)) if stopping => return,
                Ok(_) => {}
                Err(_) if stopping => return,
                Err(ConnectionError::ConnectionRefused(code)) => {
                    crate::warning!("camd.mqtt", "mqtt connect refused: {:?}", code);
                    self.pause();
                }
                Err(e) => {
                    if !reported {
                        crate::warning!("camd.mqtt", "mqtt unavailable: {}", e);
                        reported = true;
                    }
                    self.pause();
                }
            }
        }
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
        if let Some(client) = self
            .shared
            .client
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
        {
            let _ = client.try_publish(
                format!("{}/{suffix}", self.topic),
                QoS::AtMostOnce,
                true,
                value.as_bytes().to_vec(),
            );
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
        if let Some(client) = self
            .shared
            .client
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take()
        {
            let _ = client.try_disconnect();
        }
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
        let (tx, rx) = mpsc::channel();
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
}
