use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use serde_json::json;

use crate::config::Config;
use crate::db::Database;
use crate::error::Result;
use crate::push::WebPush;

pub const REQUEST_TIMEOUT_S: u64 = 10;

pub trait Alerts: Send + Sync {
    fn send(&self, title: &str, body: &str, url: &str, tag: &str) -> usize;
    fn push_subscriptions(&self) -> i64;
    fn channel_names(&self) -> Vec<String>;
}

pub trait Channel: Send + Sync {
    fn name(&self) -> &'static str;
    fn send(&self, title: &str, body: &str, url: &str) -> bool;
}

pub fn tls_provider() -> Arc<rustls::crypto::CryptoProvider> {
    static PROVIDER: OnceLock<Arc<rustls::crypto::CryptoProvider>> = OnceLock::new();
    Arc::clone(PROVIDER.get_or_init(|| Arc::new(rustls::crypto::ring::default_provider())))
}

fn v4_is_global(ip: Ipv4Addr) -> bool {
    let [a, b, c, d] = ip.octets();
    let blocked = a == 0
        || a == 10
        || (a == 100 && (64..128).contains(&b))
        || a == 127
        || (a == 169 && b == 254)
        || (a == 172 && (16..32).contains(&b))
        || (a == 192 && b == 0 && c == 0 && d != 9 && d != 10)
        || (a == 192 && b == 0 && c == 2)
        || (a == 192 && b == 168)
        || (a == 198 && (b == 18 || b == 19))
        || (a == 198 && b == 51 && c == 100)
        || (a == 203 && b == 0 && c == 113)
        || a >= 240;
    !blocked
}

fn v6_is_global(ip: Ipv6Addr) -> bool {
    if let Some(v4) = ip.to_ipv4_mapped() {
        return v4_is_global(v4);
    }
    let s = ip.segments();
    let blocked = ip.is_loopback()
        || ip.is_unspecified()
        || (s[0] & 0xfe00) == 0xfc00
        || (s[0] & 0xffc0) == 0xfe80
        || (s[0] == 0x2001 && s[1] == 0x0db8)
        || (s[0] == 0x0100 && s[1] == 0 && s[2] == 0 && s[3] == 0)
        || (s[0] == 0x2001 && s[1] < 0x0200)
        || (s[0] == 0x0064 && s[1] == 0xff9b && s[2] == 1)
        || s[0] == 0x2002;
    !blocked
}

pub fn is_global_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => v4_is_global(ip),
        IpAddr::V6(ip) => v6_is_global(ip),
    }
}

#[derive(Debug, Default)]
pub struct PublicResolver;

impl ureq::unversioned::resolver::Resolver for PublicResolver {
    fn resolve(
        &self,
        uri: &ureq::http::Uri,
        config: &ureq::config::Config,
        timeout: ureq::unversioned::transport::NextTimeout,
    ) -> std::result::Result<ureq::unversioned::resolver::ResolvedSocketAddrs, ureq::Error> {
        let all = ureq::unversioned::resolver::DefaultResolver::default()
            .resolve(uri, config, timeout)?;
        let mut public = self.empty();
        for addr in all.iter().filter(|a| is_global_ip(a.ip())) {
            public.push(*addr);
        }
        if public.is_empty() {
            crate::warning!(
                "camd.push",
                "refusing {}: it resolves to a private address",
                uri
            );
            return Err(ureq::Error::HostNotFound);
        }
        Ok(public)
    }
}

fn tls_config() -> ureq::tls::TlsConfig {
    ureq::tls::TlsConfig::builder()
        .unversioned_rustls_crypto_provider(tls_provider())
        .build()
}

pub fn push_agent() -> &'static ureq::Agent {
    static AGENT: OnceLock<ureq::Agent> = OnceLock::new();
    AGENT.get_or_init(|| {
        let config = ureq::Agent::config_builder()
            .tls_config(tls_config())
            .http_status_as_error(false)
            .max_redirects(0)
            .proxy(None)
            .timeout_global(Some(Duration::from_secs(REQUEST_TIMEOUT_S)))
            .build();
        ureq::Agent::with_parts(
            config,
            ureq::unversioned::transport::DefaultConnector::default(),
            PublicResolver,
        )
    })
}

pub fn http_agent() -> &'static ureq::Agent {
    static AGENT: OnceLock<ureq::Agent> = OnceLock::new();
    AGENT.get_or_init(|| {
        ureq::Agent::config_builder()
            .tls_config(tls_config())
            .http_status_as_error(false)
            .timeout_global(Some(Duration::from_secs(REQUEST_TIMEOUT_S)))
            .build()
            .new_agent()
    })
}

fn delivered(target: &str, answer: std::result::Result<u16, ureq::Error>) -> bool {
    match answer {
        Ok(status) if (200..300).contains(&status) => true,
        Ok(status) => {
            crate::warning!("camd.push", "{} answered {}", target, status);
            false
        }
        Err(e) => {
            crate::warning!("camd.push", "{} failed: {}", target, e);
            false
        }
    }
}

pub struct Notifich {
    pub base_url: String,
    pub api_key: String,
}

impl Notifich {
    pub fn new(base_url: &str, api_key: &str) -> Notifich {
        Notifich {
            base_url: base_url.trim_end_matches('/').to_string(),
            api_key: api_key.to_string(),
        }
    }
}

impl Channel for Notifich {
    fn name(&self) -> &'static str {
        "notifich"
    }

    fn send(&self, title: &str, body: &str, _url: &str) -> bool {
        let answer = http_agent()
            .post(format!("{}/api/notify", self.base_url))
            .header("X-API-Key", &self.api_key)
            .send_json(json!({"title": title, "body": body}))
            .map(|r| r.status().as_u16());
        delivered("notifich", answer)
    }
}

pub struct Ntfy {
    pub server: String,
    pub topic: String,
    pub token: String,
    pub origin: String,
}

impl Ntfy {
    pub fn new(topic_url: &str, token: &str, origin: &str) -> Ntfy {
        let trimmed = topic_url.trim_end_matches('/');
        let (server, topic) = trimmed.rsplit_once('/').unwrap_or(("", trimmed));
        Ntfy {
            server: server.to_string(),
            topic: topic.to_string(),
            token: token.to_string(),
            origin: origin.trim_end_matches('/').to_string(),
        }
    }
}

impl Channel for Ntfy {
    fn name(&self) -> &'static str {
        "ntfy"
    }

    fn send(&self, title: &str, body: &str, url: &str) -> bool {
        let message = json!({
            "topic": self.topic,
            "title": title,
            "message": body,
            "click": format!("{}{}", self.origin, url),
        });
        let mut request = http_agent().post(&self.server);
        if !self.token.is_empty() {
            request = request.header("Authorization", format!("Bearer {}", self.token));
        }
        let answer = request.send_json(message).map(|r| r.status().as_u16());
        delivered("ntfy", answer)
    }
}

pub struct Notifier {
    pub web: WebPush,
    pub channels: Vec<Box<dyn Channel>>,
}

impl Notifier {
    pub fn new(cfg: &Config, db: Arc<Database>) -> Result<Notifier> {
        let web = WebPush::open(db, &cfg.vapid_path(), &cfg.push_subject)?;
        let mut channels: Vec<Box<dyn Channel>> = Vec::new();
        if !cfg.notifich_url.is_empty() && !cfg.notifich_key.is_empty() {
            channels.push(Box::new(Notifich::new(
                &cfg.notifich_url,
                &cfg.notifich_key,
            )));
        }
        if !cfg.ntfy_url.is_empty() {
            channels.push(Box::new(Ntfy::new(
                &cfg.ntfy_url,
                &cfg.ntfy_token,
                &cfg.origin,
            )));
        }
        Ok(Notifier { web, channels })
    }
}

impl Alerts for Notifier {
    fn send(&self, title: &str, body: &str, url: &str, tag: &str) -> usize {
        let channels = self
            .channels
            .iter()
            .filter(|c| c.send(title, body, url))
            .count();
        channels + self.web.send_all(title, body, url, tag)
    }

    fn push_subscriptions(&self) -> i64 {
        self.web.count()
    }

    fn channel_names(&self) -> Vec<String> {
        self.channels.iter().map(|c| c.name().to_string()).collect()
    }
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use std::io::{BufRead, BufReader, Read, Write};
    use std::net::TcpListener;
    use std::sync::mpsc;

    #[test]
    fn push_deliveries_never_reach_private_addresses_or_follow_redirects() {
        use ureq::unversioned::resolver::Resolver;
        let config = ureq::Agent::config_builder().build();
        let resolve =
            |url: &str| PublicResolver.resolve(&url.parse().unwrap(), &config, Default::default());
        for private in [
            "https://127.0.0.1/x",
            "https://localhost/x",
            "https://10.1.2.3:8443/x",
            "https://[::1]/x",
            "https://2130706433/x",
        ] {
            assert!(resolve(private).is_err(), "{private}");
        }
        assert_eq!(
            resolve("https://8.8.8.8/x").unwrap()[0].ip().to_string(),
            "8.8.8.8"
        );
        let refused = push_agent().post("https://127.0.0.1:1/x").send(&b"x"[..]);
        assert!(
            matches!(refused, Err(ureq::Error::HostNotFound)),
            "{refused:?}"
        );
        assert_eq!(push_agent().config().max_redirects(), 0);
    }

    #[derive(Debug)]
    pub struct Captured {
        pub request_line: String,
        pub headers: Vec<(String, String)>,
        pub body: Vec<u8>,
    }

    impl Captured {
        pub fn header(&self, name: &str) -> Option<&str> {
            self.headers
                .iter()
                .find(|(k, _)| k.eq_ignore_ascii_case(name))
                .map(|(_, v)| v.as_str())
        }
    }

    pub fn capture_server(status: u16) -> (String, mpsc::Receiver<Captured>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { return };
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut request_line = String::new();
                reader.read_line(&mut request_line).unwrap();
                let mut headers = Vec::new();
                loop {
                    let mut line = String::new();
                    reader.read_line(&mut line).unwrap();
                    let line = line.trim_end();
                    if line.is_empty() {
                        break;
                    }
                    let (k, v) = line.split_once(':').unwrap();
                    headers.push((k.trim().to_string(), v.trim().to_string()));
                }
                let length = headers
                    .iter()
                    .find(|(k, _)| k.eq_ignore_ascii_case("content-length"))
                    .map(|(_, v)| v.parse::<usize>().unwrap())
                    .unwrap_or(0);
                let mut body = vec![0u8; length];
                reader.read_exact(&mut body).unwrap();
                let _ = write!(
                    stream,
                    "HTTP/1.1 {status} X\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                );
                let _ = tx.send(Captured {
                    request_line: request_line.trim_end().to_string(),
                    headers,
                    body,
                });
            }
        });
        (url, rx)
    }

    fn notifier(dir: &std::path::Path, cfg: Config) -> Notifier {
        let cfg = Config {
            origin: "https://cam.example.org".into(),
            ..cfg
        };
        let db = Arc::new(Database::open(&dir.join("camd.sqlite3")).unwrap());
        Notifier::new(
            &Config {
                data_dir: dir.to_path_buf(),
                ..cfg
            },
            db,
        )
        .unwrap()
    }

    #[test]
    fn notifich_counts_as_delivered() {
        let dir = tempfile::tempdir().unwrap();
        let (url, rx) = capture_server(200);
        let sender = notifier(
            dir.path(),
            Config {
                notifich_url: format!("{url}/"),
                notifich_key: "secret".into(),
                ..Config::default()
            },
        );
        assert_eq!(sender.channel_names(), ["notifich"]);
        assert_eq!(sender.send("Title", "Body", "/", "camd"), 1);
        let got = rx.recv().unwrap();
        assert_eq!(got.request_line, "POST /api/notify HTTP/1.1");
        assert_eq!(got.header("x-api-key"), Some("secret"));
        let body: serde_json::Value = serde_json::from_slice(&got.body).unwrap();
        assert_eq!(body, json!({"title": "Title", "body": "Body"}));
    }

    #[test]
    fn ntfy_posts_json_with_a_link_back() {
        let dir = tempfile::tempdir().unwrap();
        let (url, rx) = capture_server(200);
        let sender = notifier(
            dir.path(),
            Config {
                ntfy_url: format!("{url}/my-topic"),
                ntfy_token: "tk".into(),
                ..Config::default()
            },
        );
        assert_eq!(sender.send("Title", "Body", "/#/events/3", "motion"), 1);
        let got = rx.recv().unwrap();
        assert_eq!(got.request_line, "POST / HTTP/1.1");
        assert_eq!(got.header("authorization"), Some("Bearer tk"));
        let body: serde_json::Value = serde_json::from_slice(&got.body).unwrap();
        assert_eq!(
            body,
            json!({"topic": "my-topic", "title": "Title", "message": "Body", "click": "https://cam.example.org/#/events/3"})
        );
    }

    #[test]
    fn failing_channels_never_raise_and_are_not_counted() {
        let dir = tempfile::tempdir().unwrap();
        let (url, _rx) = capture_server(500);
        let sender = notifier(
            dir.path(),
            Config {
                notifich_url: url.clone(),
                notifich_key: "k".into(),
                ntfy_url: "http://127.0.0.1:1/t".into(),
                ..Config::default()
            },
        );
        assert_eq!(sender.channel_names(), ["notifich", "ntfy"]);
        assert_eq!(sender.send("a", "b", "/", "camd"), 0);
    }

    #[test]
    fn unconfigured_channels_are_not_listed() {
        let dir = tempfile::tempdir().unwrap();
        let sender = notifier(
            dir.path(),
            Config {
                notifich_url: "https://only-url".into(),
                ..Config::default()
            },
        );
        assert!(sender.channels.is_empty());
        assert_eq!(sender.push_subscriptions(), 0);
    }

    #[test]
    fn web_push_reaches_a_real_endpoint_and_gone_ones_are_dropped() {
        let dir = tempfile::tempdir().unwrap();
        let mut sender = notifier(dir.path(), Config::default());
        sender.web.agent = http_agent();
        let browser = crate::push::random_secret();
        let keys = json!({
            "p256dh": crate::auth::webauthn::b64url(&browser.public_key().to_sec1_bytes()),
            "auth": crate::auth::webauthn::b64url(&[7u8; 16]),
        });
        let (ok, ok_rx) = capture_server(201);
        let (gone, _gone_rx) = capture_server(410);
        sender
            .web
            .subscribe(&json!({"endpoint": format!("{ok}/push"), "keys": keys}))
            .unwrap();
        sender
            .web
            .subscribe(&json!({"endpoint": format!("{gone}/push"), "keys": keys}))
            .unwrap();
        assert_eq!(sender.send("T", "B", "/", "test"), 1);
        assert_eq!(sender.push_subscriptions(), 1);
        let got = ok_rx.recv().unwrap();
        assert_eq!(got.header("ttl"), Some("3600"));
        assert_eq!(got.header("content-encoding"), Some("aes128gcm"));
        let plain = crate::push::tests::decrypt(&browser, &[7u8; 16], &got.body);
        let payload: serde_json::Value = serde_json::from_slice(&plain).unwrap();
        assert_eq!(payload["tag"], "test");
    }
}
