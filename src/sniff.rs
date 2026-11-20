use std::io;
use std::net::Ipv4Addr;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::sync::Arc;

use crate::presence::Presence;

const ETHERTYPE_IPV4: u16 = 0x0800;
const ETHERTYPE_ARP: u16 = 0x0806;
const ETHERTYPE_OFFSET: u32 = 12;
const IPV4_SOURCE_OFFSET: u32 = 26;
const ARP_SENDER_OFFSET: u32 = 28;
const IPV4_MIN_FRAME: usize = 34;
const ARP_MIN_FRAME: usize = 42;
const SNAP_BYTES: u32 = 64;
const MAX_WATCHED: usize = 64;
const RECEIVE_TIMEOUT_S: libc::time_t = 1;

const LD_H_ABS: u16 = (libc::BPF_LD | libc::BPF_H | libc::BPF_ABS) as u16;
const LD_W_ABS: u16 = (libc::BPF_LD | libc::BPF_W | libc::BPF_ABS) as u16;
const JEQ_K: u16 = (libc::BPF_JMP | libc::BPF_JEQ | libc::BPF_K) as u16;
const RET_K: u16 = (libc::BPF_RET | libc::BPF_K) as u16;

pub fn source_address(frame: &[u8]) -> Option<Ipv4Addr> {
    if frame.len() < IPV4_MIN_FRAME {
        return None;
    }
    let ethertype = u16::from_be_bytes([frame[12], frame[13]]);
    let at = match ethertype {
        ETHERTYPE_IPV4 => IPV4_SOURCE_OFFSET as usize,
        ETHERTYPE_ARP if frame.len() >= ARP_MIN_FRAME => ARP_SENDER_OFFSET as usize,
        _ => return None,
    };
    let address = Ipv4Addr::new(frame[at], frame[at + 1], frame[at + 2], frame[at + 3]);
    (!address.is_unspecified()).then_some(address)
}

fn instruction(code: u16, jt: u8, jf: u8, k: u32) -> libc::sock_filter {
    libc::sock_filter { code, jt, jf, k }
}

pub fn filter_program(addresses: &[Ipv4Addr]) -> Vec<libc::sock_filter> {
    let n = addresses.len().min(MAX_WATCHED);
    let addresses = &addresses[..n];
    let accept = 7 + 2 * n;
    let mut program = Vec::with_capacity(accept + 1);
    program.push(instruction(LD_H_ABS, 0, 0, ETHERTYPE_OFFSET));
    program.push(instruction(JEQ_K, 0, (n + 2) as u8, ETHERTYPE_IPV4 as u32));
    program.push(instruction(LD_W_ABS, 0, 0, IPV4_SOURCE_OFFSET));
    for (k, address) in addresses.iter().enumerate() {
        program.push(instruction(
            JEQ_K,
            (3 + 2 * n - k) as u8,
            0,
            u32::from(*address),
        ));
    }
    program.push(instruction(RET_K, 0, 0, 0));
    program.push(instruction(JEQ_K, 0, (n + 1) as u8, ETHERTYPE_ARP as u32));
    program.push(instruction(LD_W_ABS, 0, 0, ARP_SENDER_OFFSET));
    for (k, address) in addresses.iter().enumerate() {
        program.push(instruction(JEQ_K, (n - k) as u8, 0, u32::from(*address)));
    }
    program.push(instruction(RET_K, 0, 0, 0));
    program.push(instruction(RET_K, 0, 0, SNAP_BYTES));
    program
}

fn open(addresses: &[Ipv4Addr]) -> io::Result<OwnedFd> {
    let protocol = i32::from((libc::ETH_P_ALL as u16).to_be());
    let raw = unsafe {
        libc::socket(
            libc::AF_PACKET,
            libc::SOCK_RAW | libc::SOCK_CLOEXEC,
            protocol,
        )
    };
    if raw < 0 {
        return Err(io::Error::last_os_error());
    }
    let fd = unsafe { OwnedFd::from_raw_fd(raw) };
    let program = filter_program(addresses);
    let filter = libc::sock_fprog {
        len: program.len() as u16,
        filter: program.as_ptr().cast_mut(),
    };
    let attached = unsafe {
        libc::setsockopt(
            fd.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_ATTACH_FILTER,
            std::ptr::from_ref(&filter).cast(),
            std::mem::size_of::<libc::sock_fprog>() as libc::socklen_t,
        )
    };
    if attached < 0 {
        return Err(io::Error::last_os_error());
    }
    let timeout = libc::timeval {
        tv_sec: RECEIVE_TIMEOUT_S,
        tv_usec: 0,
    };
    let configured = unsafe {
        libc::setsockopt(
            fd.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_RCVTIMEO,
            std::ptr::from_ref(&timeout).cast(),
            std::mem::size_of::<libc::timeval>() as libc::socklen_t,
        )
    };
    if configured < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(fd)
}

fn listen(presence: &Presence, fd: &OwnedFd, watched: &[Ipv4Addr]) {
    let mut buffer = [0u8; SNAP_BYTES as usize];
    while !presence.is_halted() {
        let received =
            unsafe { libc::recv(fd.as_raw_fd(), buffer.as_mut_ptr().cast(), buffer.len(), 0) };
        if received < 0 {
            match io::Error::last_os_error().raw_os_error() {
                Some(libc::EAGAIN) | Some(libc::EINTR) => continue,
                _ => return,
            }
        }
        let frame = &buffer[..received as usize];
        if source_address(frame).is_some_and(|address| watched.contains(&address)) {
            presence.mark_seen();
        }
    }
}

pub fn start(presence: &Arc<Presence>) -> io::Result<bool> {
    let watched: Vec<Ipv4Addr> = presence
        .devices
        .iter()
        .filter_map(|device| device.parse().ok())
        .take(MAX_WATCHED)
        .collect();
    if watched.is_empty() {
        return Ok(false);
    }
    let fd = open(&watched)?;
    let presence = Arc::clone(presence);
    std::thread::Builder::new()
        .name("presence-listen".into())
        .spawn(move || listen(&presence, &fd, &watched))?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clock;
    use std::time::{Duration, Instant};

    fn frame(ethertype: u16, source_at: usize, source: [u8; 4], length: usize) -> Vec<u8> {
        let mut bytes = vec![0u8; length];
        bytes[12..14].copy_from_slice(&ethertype.to_be_bytes());
        bytes[source_at..source_at + 4].copy_from_slice(&source);
        bytes
    }

    fn ipv4_frame(source: [u8; 4]) -> Vec<u8> {
        frame(ETHERTYPE_IPV4, 26, source, 60)
    }

    fn arp_frame(sender: [u8; 4]) -> Vec<u8> {
        frame(ETHERTYPE_ARP, 28, sender, 60)
    }

    fn run_filter(program: &[libc::sock_filter], packet: &[u8]) -> u32 {
        let mut accumulator = 0u32;
        let mut pc = 0usize;
        loop {
            let step = &program[pc];
            pc += 1;
            match step.code {
                LD_H_ABS => {
                    let at = step.k as usize;
                    accumulator = u32::from(u16::from_be_bytes([packet[at], packet[at + 1]]));
                }
                LD_W_ABS => {
                    let at = step.k as usize;
                    accumulator = u32::from_be_bytes([
                        packet[at],
                        packet[at + 1],
                        packet[at + 2],
                        packet[at + 3],
                    ]);
                }
                JEQ_K => {
                    pc += if accumulator == step.k {
                        step.jt as usize
                    } else {
                        step.jf as usize
                    };
                }
                RET_K => return step.k,
                other => panic!("unexpected opcode {other}"),
            }
        }
    }

    #[test]
    fn the_source_of_an_ipv4_frame_is_read() {
        let packet = ipv4_frame([192, 168, 1, 60]);
        assert_eq!(
            source_address(&packet),
            Some(Ipv4Addr::new(192, 168, 1, 60))
        );
    }

    #[test]
    fn the_sender_of_an_arp_frame_is_read_unless_it_is_a_probe() {
        let packet = arp_frame([192, 168, 1, 60]);
        assert_eq!(
            source_address(&packet),
            Some(Ipv4Addr::new(192, 168, 1, 60))
        );
        assert_eq!(source_address(&arp_frame([0, 0, 0, 0])), None);
    }

    #[test]
    fn short_and_unrelated_frames_have_no_source() {
        assert_eq!(source_address(&[0u8; 10]), None);
        assert_eq!(
            source_address(&frame(0x86dd, 26, [192, 168, 1, 60], 60)),
            None
        );
        assert_eq!(
            source_address(&frame(ETHERTYPE_ARP, 28, [192, 168, 1, 60], 40)),
            None
        );
    }

    #[test]
    fn the_filter_accepts_only_watched_senders_in_ipv4_and_arp() {
        let watched = [
            Ipv4Addr::new(192, 168, 1, 60),
            Ipv4Addr::new(192, 168, 1, 61),
        ];
        let program = filter_program(&watched);
        for source in [[192, 168, 1, 60], [192, 168, 1, 61]] {
            assert_eq!(run_filter(&program, &ipv4_frame(source)), SNAP_BYTES);
            assert_eq!(run_filter(&program, &arp_frame(source)), SNAP_BYTES);
        }
        for source in [[192, 168, 1, 62], [192, 168, 1, 73], [10, 0, 0, 60]] {
            assert_eq!(run_filter(&program, &ipv4_frame(source)), 0);
            assert_eq!(run_filter(&program, &arp_frame(source)), 0);
        }
        assert_eq!(
            run_filter(&program, &frame(0x86dd, 26, [192, 168, 1, 60], 60)),
            0
        );
        assert_eq!(
            run_filter(&program, &frame(0x86dd, 28, [192, 168, 1, 60], 60)),
            0
        );
    }

    #[test]
    fn the_filter_works_for_a_single_address_and_for_the_maximum() {
        let one = [Ipv4Addr::new(10, 1, 2, 3)];
        let program = filter_program(&one);
        assert_eq!(run_filter(&program, &ipv4_frame([10, 1, 2, 3])), SNAP_BYTES);
        assert_eq!(run_filter(&program, &arp_frame([10, 1, 2, 3])), SNAP_BYTES);
        assert_eq!(run_filter(&program, &ipv4_frame([10, 1, 2, 4])), 0);
        let many: Vec<Ipv4Addr> = (0..100).map(|i| Ipv4Addr::new(10, 0, 0, i)).collect();
        let program = filter_program(&many);
        assert_eq!(
            run_filter(&program, &ipv4_frame([10, 0, 0, 63])),
            SNAP_BYTES
        );
        assert_eq!(run_filter(&program, &arp_frame([10, 0, 0, 0])), SNAP_BYTES);
        assert_eq!(run_filter(&program, &ipv4_frame([10, 0, 0, 64])), 0);
    }

    #[test]
    fn devices_that_are_not_ipv4_addresses_start_no_listener() {
        let presence = Presence::new(vec!["not-an-address".into(), "fe80::1".into()], 30.0);
        assert!(!start(&presence).unwrap());
        let presence = Presence::new(vec![], 30.0);
        assert!(!start(&presence).unwrap());
    }

    #[test]
    fn a_frame_from_a_watched_device_marks_it_seen() {
        let presence =
            Presence::with_interval(vec!["127.0.0.1".into()], 30.0, Duration::from_secs(3600));
        match start(&presence) {
            Ok(true) => {}
            Ok(false) => panic!("a valid address must start a listener"),
            Err(e) if e.raw_os_error() == Some(libc::EPERM) => return,
            Err(e) => panic!("cannot open the listener: {e}"),
        }
        presence.set_last_seen(0.0);
        assert!(!presence.is_home());
        let socket = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
        let deadline = Instant::now() + Duration::from_secs(3);
        while !presence.is_home() && Instant::now() < deadline {
            socket.send_to(b"x", "127.0.0.1:9").unwrap();
            std::thread::sleep(Duration::from_millis(50));
        }
        presence.stop();
        assert!(presence.is_home());
        assert!(presence.last_seen() > clock::now() - 5.0);
    }
}
