use std::io::{BufRead, IsTerminal, Write};
use std::sync::Arc;

use crate::auth::Auth;
use crate::clock;
use crate::config;
use crate::db::Database;
use crate::error::{Error, Result};

pub const COMMANDS: [&str; 4] = ["enroll", "password", "revoke-all", "devices"];
pub const USAGE: &str = "usage: camd ctl [-h] [--version] {enroll,password,revoke-all,devices}";

pub trait Console {
    fn out(&mut self, line: &str);
    fn err(&mut self, line: &str);
    fn secret(&mut self, prompt: &str) -> String;
}

pub struct Terminal;

impl Console for Terminal {
    fn out(&mut self, line: &str) {
        let _ = writeln!(std::io::stdout(), "{line}");
    }

    fn err(&mut self, line: &str) {
        let _ = writeln!(std::io::stderr(), "{line}");
    }

    fn secret(&mut self, prompt: &str) -> String {
        read_secret(prompt)
    }
}

fn read_line() -> String {
    let mut line = String::new();
    let _ = std::io::stdin().lock().read_line(&mut line);
    line.trim_end_matches(['\n', '\r']).to_string()
}

fn read_secret(prompt: &str) -> String {
    let stdin = std::io::stdin();
    if !stdin.is_terminal() {
        eprint!("{prompt}");
        let _ = std::io::stderr().flush();
        return read_line();
    }
    eprint!("{prompt}");
    let _ = std::io::stderr().flush();
    let fd = libc::STDIN_FILENO;
    let mut saved: libc::termios = unsafe { std::mem::zeroed() };
    let echo_off = unsafe { libc::tcgetattr(fd, &mut saved) } == 0 && {
        let mut quiet = saved;
        quiet.c_lflag &= !libc::ECHO;
        quiet.c_lflag |= libc::ECHONL;
        unsafe { libc::tcsetattr(fd, libc::TCSANOW, &quiet) == 0 }
    };
    let line = read_line();
    if echo_off {
        unsafe { libc::tcsetattr(fd, libc::TCSANOW, &saved) };
    }
    line
}

pub fn device_lines(auth: &Auth) -> Result<Vec<String>> {
    let mut lines = Vec::new();
    for credential in auth.credentials()? {
        lines.push(format!(
            "passkey  {:<20} {}",
            credential.name,
            clock::local(credential.created).date_minute()
        ));
    }
    if auth.has_password()? {
        lines.push("password set".to_string());
    }
    for session in auth.sessions()? {
        let agent: String = session.user_agent.chars().take(60).collect();
        lines.push(format!("session  {:<4} {}", session.id, agent));
    }
    Ok(lines)
}

pub fn execute(command: &str, auth: &Auth, console: &mut dyn Console) -> Result<()> {
    match command {
        "enroll" => console.out(&format!(
            "Enrollment code (valid 10 minutes): {}",
            auth.new_enroll_code()?
        )),
        "password" => {
            let password = console.secret("New password: ");
            if password != console.secret("Repeat it: ") {
                return Err(Error::invalid("the two passwords differ"));
            }
            auth.set_password(&password)?;
            console.out("Password saved");
        }
        "revoke-all" => {
            auth.revoke_all()?;
            console.out("All sessions revoked");
        }
        "devices" => {
            for line in device_lines(auth)? {
                console.out(&line);
            }
        }
        other => return Err(Error::invalid(format!("unknown command: {other}"))),
    }
    Ok(())
}

pub fn run(args: &[String], console: &mut dyn Console) -> i32 {
    let mut command = None;
    for arg in args {
        match arg.as_str() {
            "-h" | "--help" => {
                console.out(USAGE);
                console.out("");
                console.out("Local administration of the camera service");
                return 0;
            }
            "--version" => {
                console.out(&format!("camd {}", crate::VERSION));
                return 0;
            }
            word if command.is_none() && COMMANDS.contains(&word) => {
                command = Some(word.to_string())
            }
            word => {
                console.err(USAGE);
                let reason = if command.is_some() || word.starts_with('-') {
                    format!("unrecognized arguments: {word}")
                } else {
                    format!(
                        "argument command: invalid choice: '{word}' (choose from enroll, password, revoke-all, devices)"
                    )
                };
                console.err(&format!("camd ctl: error: {reason}"));
                return 2;
            }
        }
    }
    let Some(command) = command else {
        console.err(USAGE);
        console.err("camd ctl: error: the following arguments are required: command");
        return 2;
    };
    let outcome = config::load(None).and_then(|cfg| {
        let db = Arc::new(Database::open(&cfg.db_path())?);
        let auth = Auth::new(&cfg, db);
        execute(&command, &auth, console)
    });
    match outcome {
        Ok(()) => 0,
        Err(e) => {
            console.err(e.message());
            1
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;

    #[derive(Default)]
    struct Script {
        out: Vec<String>,
        err: Vec<String>,
        secrets: Vec<String>,
    }

    impl Console for Script {
        fn out(&mut self, line: &str) {
            self.out.push(line.to_string());
        }

        fn err(&mut self, line: &str) {
            self.err.push(line.to_string());
        }

        fn secret(&mut self, _prompt: &str) -> String {
            self.secrets.remove(0)
        }
    }

    fn auth() -> (tempfile::TempDir, Auth) {
        let dir = tempfile::tempdir().unwrap();
        let cfg = Config::with_data_dir(dir.path());
        let db = Arc::new(Database::open(&cfg.db_path()).unwrap());
        (dir, Auth::new(&cfg, db))
    }

    #[test]
    fn enroll_prints_a_usable_code() {
        let (_d, auth) = auth();
        let mut console = Script::default();
        execute("enroll", &auth, &mut console).unwrap();
        let code = console.out[0]
            .strip_prefix("Enrollment code (valid 10 minutes): ")
            .unwrap();
        auth.check_code(code).unwrap();
    }

    #[test]
    fn password_is_asked_twice_and_checked() {
        let (_d, auth) = auth();
        let mut console = Script {
            secrets: vec!["a long enough one".into(), "different".into()],
            ..Default::default()
        };
        assert_eq!(
            execute("password", &auth, &mut console)
                .unwrap_err()
                .message(),
            "the two passwords differ"
        );
        console.secrets = vec!["short".into(), "short".into()];
        assert_eq!(
            execute("password", &auth, &mut console)
                .unwrap_err()
                .message(),
            "password must have at least 10 characters"
        );
        console.secrets = vec!["a long enough one".into(), "a long enough one".into()];
        execute("password", &auth, &mut console).unwrap();
        assert_eq!(console.out, vec!["Password saved"]);
        assert!(auth.login_password("a long enough one", "ua").is_ok());
    }

    #[test]
    fn devices_and_revoke_all() {
        let (_d, auth) = auth();
        auth.db()
            .insert_credential(&crate::db::CredentialRow {
                id: "abc".into(),
                public_key: vec![1],
                sign_count: 0,
                name: "phone".into(),
                created: clock::mktime(2026, 11, 14, 9, 5, 0),
            })
            .unwrap();
        auth.set_password("a long enough one").unwrap();
        auth.create_session(&format!("Mozilla/5.0 {}", "x".repeat(80)))
            .unwrap();
        let lines = device_lines(&auth).unwrap();
        assert_eq!(
            lines[0],
            format!("passkey  {:<20} 2026-11-14 09:05", "phone")
        );
        assert_eq!(lines[1], "password set");
        assert!(lines[2].starts_with("session  1    Mozilla/5.0 "));
        assert_eq!(lines[2].len(), "session  1    ".len() + 60);
        let mut console = Script::default();
        execute("revoke-all", &auth, &mut console).unwrap();
        assert_eq!(console.out, vec!["All sessions revoked"]);
        assert!(auth.sessions().unwrap().is_empty());
    }

    #[test]
    fn bad_arguments_exit_with_usage() {
        let mut console = Script::default();
        assert_eq!(run(&["nope".into()], &mut console), 2);
        assert!(console.err[1].contains("invalid choice: 'nope'"));
        assert_eq!(run(&[], &mut console), 2);
        let mut console = Script::default();
        assert_eq!(run(&["--version".into()], &mut console), 0);
        assert_eq!(console.out, vec![format!("camd {}", crate::VERSION)]);
    }
}
