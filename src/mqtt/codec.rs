pub const MAX_REMAINING: usize = 268_435_455;
pub const MAX_INCOMING: usize = 64 * 1024;

const CONNECT: u8 = 0x10;
const CONNACK: u8 = 2;
const PUBLISH: u8 = 3;
const PUBACK: u8 = 4;
const PUBREC: u8 = 5;
const PUBREL: u8 = 6;
const PUBCOMP: u8 = 7;
const SUBSCRIBE: u8 = 0x82;
const SUBACK: u8 = 9;
const UNSUBACK: u8 = 11;
const PINGRESP: u8 = 13;

pub const PINGREQ: [u8; 2] = [0xc0, 0];
pub const DISCONNECT: [u8; 2] = [0xe0, 0];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Malformed {
    LengthOverflow,
    TooLarge(usize),
    Truncated,
    BadString,
    BadQos,
    BadFlags(u8),
    Unexpected(u8),
    TooLong,
}

impl std::fmt::Display for Malformed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Malformed::LengthOverflow => write!(f, "remaining length longer than 4 bytes"),
            Malformed::TooLarge(len) => write!(f, "packet of {len} bytes is too large"),
            Malformed::Truncated => write!(f, "packet shorter than its fields"),
            Malformed::BadString => write!(f, "string is not UTF-8"),
            Malformed::BadQos => write!(f, "invalid QoS"),
            Malformed::BadFlags(first) => write!(f, "invalid flags in header 0x{first:02x}"),
            Malformed::Unexpected(kind) => write!(f, "unexpected packet type {kind}"),
            Malformed::TooLong => write!(f, "field too long to encode"),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConnectReturnCode {
    Success,
    RefusedProtocolVersion,
    BadClientId,
    ServiceUnavailable,
    BadUserNamePassword,
    NotAuthorized,
    Other(u8),
}

impl From<u8> for ConnectReturnCode {
    fn from(code: u8) -> Self {
        match code {
            0 => ConnectReturnCode::Success,
            1 => ConnectReturnCode::RefusedProtocolVersion,
            2 => ConnectReturnCode::BadClientId,
            3 => ConnectReturnCode::ServiceUnavailable,
            4 => ConnectReturnCode::BadUserNamePassword,
            5 => ConnectReturnCode::NotAuthorized,
            other => ConnectReturnCode::Other(other),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Packet {
    ConnAck {
        session_present: bool,
        code: ConnectReturnCode,
    },
    Publish {
        topic: String,
        payload: Vec<u8>,
        qos: u8,
        retain: bool,
        id: Option<u16>,
    },
    PubAck(u16),
    PubRec(u16),
    PubRel(u16),
    PubComp(u16),
    SubAck {
        id: u16,
        codes: Vec<u8>,
    },
    UnsubAck(u16),
    PingResp,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Will {
    pub topic: String,
    pub payload: Vec<u8>,
    pub retain: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Connect {
    pub client_id: String,
    pub keep_alive: u16,
    pub username: Option<String>,
    pub password: Option<String>,
    pub will: Option<Will>,
}

fn put_length(out: &mut Vec<u8>, mut len: usize) -> Result<(), Malformed> {
    if len > MAX_REMAINING {
        return Err(Malformed::TooLong);
    }
    loop {
        let mut byte = (len % 128) as u8;
        len /= 128;
        if len > 0 {
            byte |= 0x80;
        }
        out.push(byte);
        if len == 0 {
            return Ok(());
        }
    }
}

fn put_bytes(out: &mut Vec<u8>, bytes: &[u8]) -> Result<(), Malformed> {
    let len = u16::try_from(bytes.len()).map_err(|_| Malformed::TooLong)?;
    out.extend_from_slice(&len.to_be_bytes());
    out.extend_from_slice(bytes);
    Ok(())
}

fn frame(first: u8, body: &[u8]) -> Result<Vec<u8>, Malformed> {
    let mut out = Vec::with_capacity(body.len() + 5);
    out.push(first);
    put_length(&mut out, body.len())?;
    out.extend_from_slice(body);
    Ok(out)
}

pub fn connect(packet: &Connect) -> Result<Vec<u8>, Malformed> {
    let mut flags = 0x02;
    let mut body = Vec::new();
    put_bytes(&mut body, b"MQTT")?;
    body.push(4);
    let flags_at = body.len();
    body.push(0);
    body.extend_from_slice(&packet.keep_alive.to_be_bytes());
    put_bytes(&mut body, packet.client_id.as_bytes())?;
    if let Some(will) = &packet.will {
        flags |= 0x04;
        if will.retain {
            flags |= 0x20;
        }
        put_bytes(&mut body, will.topic.as_bytes())?;
        put_bytes(&mut body, &will.payload)?;
    }
    if let Some(username) = &packet.username {
        flags |= 0x80;
        put_bytes(&mut body, username.as_bytes())?;
        if let Some(password) = &packet.password {
            flags |= 0x40;
            put_bytes(&mut body, password.as_bytes())?;
        }
    }
    body[flags_at] = flags;
    frame(CONNECT, &body)
}

pub fn publish(topic: &str, payload: &[u8], retain: bool) -> Result<Vec<u8>, Malformed> {
    let mut body = Vec::with_capacity(topic.len() + payload.len() + 2);
    put_bytes(&mut body, topic.as_bytes())?;
    body.extend_from_slice(payload);
    frame(0x30 | u8::from(retain), &body)
}

pub fn subscribe(id: u16, topic: &str) -> Result<Vec<u8>, Malformed> {
    let mut body = id.to_be_bytes().to_vec();
    put_bytes(&mut body, topic.as_bytes())?;
    body.push(0);
    frame(SUBSCRIBE, &body)
}

pub fn puback(id: u16) -> [u8; 4] {
    let [hi, lo] = id.to_be_bytes();
    [PUBACK << 4, 2, hi, lo]
}

pub fn pubrec(id: u16) -> [u8; 4] {
    let [hi, lo] = id.to_be_bytes();
    [PUBREC << 4, 2, hi, lo]
}

pub fn pubcomp(id: u16) -> [u8; 4] {
    let [hi, lo] = id.to_be_bytes();
    [PUBCOMP << 4, 2, hi, lo]
}

pub fn remaining_length(input: &[u8]) -> Result<Option<(usize, usize)>, Malformed> {
    let mut len = 0usize;
    for (i, byte) in input.iter().take(4).enumerate() {
        len |= usize::from(byte & 0x7f) << (7 * i);
        if byte & 0x80 == 0 {
            return Ok(Some((len, i + 1)));
        }
    }
    if input.len() >= 4 {
        Err(Malformed::LengthOverflow)
    } else {
        Ok(None)
    }
}

fn id_of(body: &[u8]) -> Result<u16, Malformed> {
    match body {
        [hi, lo] => Ok(u16::from_be_bytes([*hi, *lo])),
        _ => Err(Malformed::Truncated),
    }
}

fn decode_publish(flags: u8, body: &[u8]) -> Result<Packet, Malformed> {
    let qos = (flags >> 1) & 3;
    if qos == 3 {
        return Err(Malformed::BadQos);
    }
    let [hi, lo, rest @ ..] = body else {
        return Err(Malformed::Truncated);
    };
    let len = usize::from(u16::from_be_bytes([*hi, *lo]));
    let topic = rest.get(..len).ok_or(Malformed::Truncated)?;
    let topic = std::str::from_utf8(topic).map_err(|_| Malformed::BadString)?;
    let mut rest = &rest[len..];
    let id = if qos > 0 {
        let [hi, lo, tail @ ..] = rest else {
            return Err(Malformed::Truncated);
        };
        rest = tail;
        Some(u16::from_be_bytes([*hi, *lo]))
    } else {
        None
    };
    Ok(Packet::Publish {
        topic: topic.to_string(),
        payload: rest.to_vec(),
        qos,
        retain: flags & 1 == 1,
        id,
    })
}

pub fn decode(input: &[u8]) -> Result<Option<(Packet, usize)>, Malformed> {
    let Some(&first) = input.first() else {
        return Ok(None);
    };
    let Some((len, header)) = remaining_length(&input[1..])? else {
        return Ok(None);
    };
    if len > MAX_INCOMING {
        return Err(Malformed::TooLarge(len));
    }
    let end = 1 + header + len;
    let Some(body) = input.get(1 + header..end) else {
        return Ok(None);
    };
    let (kind, flags) = (first >> 4, first & 0x0f);
    let expected_flags = if kind == PUBREL { 2 } else { 0 };
    if kind != PUBLISH && flags != expected_flags {
        return Err(Malformed::BadFlags(first));
    }
    let packet = match kind {
        CONNACK => match body {
            [ack, code] => Packet::ConnAck {
                session_present: ack & 1 == 1,
                code: ConnectReturnCode::from(*code),
            },
            _ => return Err(Malformed::Truncated),
        },
        PUBLISH => decode_publish(flags, body)?,
        PUBACK => Packet::PubAck(id_of(body)?),
        PUBREC => Packet::PubRec(id_of(body)?),
        PUBREL => Packet::PubRel(id_of(body)?),
        PUBCOMP => Packet::PubComp(id_of(body)?),
        SUBACK => match body {
            [hi, lo, codes @ ..] if !codes.is_empty() => Packet::SubAck {
                id: u16::from_be_bytes([*hi, *lo]),
                codes: codes.to_vec(),
            },
            _ => return Err(Malformed::Truncated),
        },
        UNSUBACK => Packet::UnsubAck(id_of(body)?),
        PINGRESP if body.is_empty() => Packet::PingResp,
        PINGRESP => return Err(Malformed::Truncated),
        other => return Err(Malformed::Unexpected(other)),
    };
    Ok(Some((packet, end)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn length(len: usize) -> Vec<u8> {
        let mut out = Vec::new();
        put_length(&mut out, len).unwrap();
        out
    }

    #[test]
    fn remaining_length_edges() {
        for (len, bytes) in [
            (0, vec![0x00]),
            (127, vec![0x7f]),
            (128, vec![0x80, 0x01]),
            (16_383, vec![0xff, 0x7f]),
            (16_384, vec![0x80, 0x80, 0x01]),
            (2_097_151, vec![0xff, 0xff, 0x7f]),
            (2_097_152, vec![0x80, 0x80, 0x80, 0x01]),
            (MAX_REMAINING, vec![0xff, 0xff, 0xff, 0x7f]),
        ] {
            assert_eq!(length(len), bytes, "{len}");
            assert_eq!(remaining_length(&bytes), Ok(Some((len, bytes.len()))));
            assert_eq!(remaining_length(&bytes[..bytes.len() - 1]), Ok(None));
        }
        assert_eq!(
            put_length(&mut Vec::new(), MAX_REMAINING + 1),
            Err(Malformed::TooLong)
        );
        assert_eq!(
            remaining_length(&[0xff, 0xff, 0xff, 0xff, 0x01]),
            Err(Malformed::LengthOverflow)
        );
        assert_eq!(remaining_length(&[]), Ok(None));
    }

    #[test]
    fn connect_carries_will_and_credentials() {
        let packet = Connect {
            client_id: "camd-cam".into(),
            keep_alive: 30,
            username: Some("u".into()),
            password: Some("p".into()),
            will: Some(Will {
                topic: "cam/status".into(),
                payload: b"offline".to_vec(),
                retain: true,
            }),
        };
        let mut expected = vec![0x10, 47, 0, 4, b'M', b'Q', b'T', b'T', 4, 0xe6, 0, 30];
        expected.extend_from_slice(b"\0\x08camd-cam\0\x0acam/status\0\x07offline\0\x01u\0\x01p");
        assert_eq!(connect(&packet).unwrap(), expected);
        let bare = Connect {
            client_id: "c".into(),
            keep_alive: 30,
            username: None,
            password: None,
            will: None,
        };
        assert_eq!(
            connect(&bare).unwrap(),
            [
                0x10, 13, 0, 4, b'M', b'Q', b'T', b'T', 4, 0x02, 0, 30, 0, 1, b'c'
            ]
        );
        let user_only = Connect {
            username: Some("u".into()),
            ..bare
        };
        assert_eq!(connect(&user_only).unwrap()[9], 0x82);
    }

    #[test]
    fn client_packets_encode() {
        assert_eq!(
            publish("a/b", b"ON", true).unwrap(),
            [0x31, 7, 0, 3, b'a', b'/', b'b', b'O', b'N']
        );
        assert_eq!(publish("t", b"", false).unwrap(), [0x30, 3, 0, 1, b't']);
        assert_eq!(
            subscribe(0x0102, "a/set").unwrap(),
            [0x82, 10, 1, 2, 0, 5, b'a', b'/', b's', b'e', b't', 0]
        );
        assert_eq!(puback(7), [0x40, 2, 0, 7]);
        assert_eq!(pubrec(0x1234), [0x50, 2, 0x12, 0x34]);
        assert_eq!(pubcomp(1), [0x70, 2, 0, 1]);
        assert_eq!(PINGREQ, [0xc0, 0]);
        assert_eq!(DISCONNECT, [0xe0, 0]);
        let long = "x".repeat(70_000);
        assert_eq!(publish(&long, b"", true), Err(Malformed::TooLong));
        assert_eq!(subscribe(1, &long), Err(Malformed::TooLong));
        let big = publish("t", &vec![7u8; 20_000], true).unwrap();
        assert_eq!(&big[..4], [0x31, 0xa3, 0x9c, 0x01]);
    }

    #[test]
    fn server_packets_decode() {
        let cases: Vec<(&[u8], Packet)> = vec![
            (
                &[0x20, 2, 0, 0],
                Packet::ConnAck {
                    session_present: false,
                    code: ConnectReturnCode::Success,
                },
            ),
            (
                &[0x20, 2, 1, 5],
                Packet::ConnAck {
                    session_present: true,
                    code: ConnectReturnCode::NotAuthorized,
                },
            ),
            (
                &[0x31, 7, 0, 3, b'a', b'/', b'b', b'O', b'N'],
                Packet::Publish {
                    topic: "a/b".into(),
                    payload: b"ON".to_vec(),
                    qos: 0,
                    retain: true,
                    id: None,
                },
            ),
            (
                &[0x32, 6, 0, 1, b't', 0, 9, b'x'],
                Packet::Publish {
                    topic: "t".into(),
                    payload: b"x".to_vec(),
                    qos: 1,
                    retain: false,
                    id: Some(9),
                },
            ),
            (&[0x40, 2, 0, 1], Packet::PubAck(1)),
            (&[0x50, 2, 0, 2], Packet::PubRec(2)),
            (&[0x62, 2, 0, 3], Packet::PubRel(3)),
            (&[0x70, 2, 0, 4], Packet::PubComp(4)),
            (
                &[0x90, 3, 0, 5, 0],
                Packet::SubAck {
                    id: 5,
                    codes: vec![0],
                },
            ),
            (&[0xb0, 2, 0, 6], Packet::UnsubAck(6)),
            (&[0xd0, 0], Packet::PingResp),
        ];
        for (bytes, packet) in cases {
            let mut stream = bytes.to_vec();
            stream.extend_from_slice(&[0xd0, 0]);
            assert_eq!(decode(&stream), Ok(Some((packet, bytes.len()))));
            for cut in 0..bytes.len() {
                assert_eq!(decode(&bytes[..cut]), Ok(None), "{bytes:?} cut at {cut}");
            }
        }
        let published = publish("cam/mode/set", b"armed", false).unwrap();
        assert_eq!(
            decode(&published).unwrap().unwrap().0,
            Packet::Publish {
                topic: "cam/mode/set".into(),
                payload: b"armed".to_vec(),
                qos: 0,
                retain: false,
                id: None,
            }
        );
    }

    #[test]
    fn malformed_packets_are_errors() {
        let cases: Vec<(&[u8], Malformed)> = vec![
            (&[0x20, 1, 0], Malformed::Truncated),
            (&[0x21, 2, 0, 0], Malformed::BadFlags(0x21)),
            (&[0x60, 2, 0, 0], Malformed::BadFlags(0x60)),
            (&[0x36, 3, 0, 1, b't'], Malformed::BadQos),
            (&[0x30, 1, 0], Malformed::Truncated),
            (&[0x30, 3, 0, 5, b't'], Malformed::Truncated),
            (&[0x32, 3, 0, 1, b't'], Malformed::Truncated),
            (&[0x30, 4, 0, 2, 0xff, 0xfe], Malformed::BadString),
            (&[0x90, 2, 0, 1], Malformed::Truncated),
            (&[0xd0, 1, 0], Malformed::Truncated),
            (&[0x10, 0], Malformed::Unexpected(1)),
            (&[0xc0, 0], Malformed::Unexpected(12)),
            (&[0xe0, 0], Malformed::Unexpected(14)),
            (&[0x00, 0], Malformed::Unexpected(0)),
            (&[0x30, 0xff, 0xff, 0xff, 0xff], Malformed::LengthOverflow),
            (&[0x30, 0x81, 0x80, 0x04], Malformed::TooLarge(65_537)),
        ];
        for (bytes, error) in cases {
            assert_eq!(decode(bytes), Err(error), "{bytes:?}");
        }
    }

    #[test]
    fn arbitrary_input_never_panics() {
        let mut state = 0x2545_f491_4f6c_dd1du64;
        let mut next = || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state
        };
        for _ in 0..20_000 {
            let len = (next() % 24) as usize;
            let mut bytes: Vec<u8> = (0..len).map(|_| next() as u8).collect();
            if let Some(length) = bytes.get_mut(1) {
                *length %= 24;
            }
            let mut rest = bytes.as_slice();
            while let Ok(Some((_, used))) = decode(rest) {
                assert!(used > 0 && used <= rest.len());
                rest = &rest[used..];
            }
        }
    }
}
