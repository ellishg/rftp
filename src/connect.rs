use crate::utils::{ErrorKind, Result};
use base64;
use dirs::home_dir;
use rpassword::prompt_password_stdout;
use std::collections::HashSet;
use std::io::{stdin, stdout, Write};
use std::net::TcpStream;

/// Create an authenticated `ssh2::Session`.
pub fn create_session(
    destination: &str,
    username: &str,
    port: Option<&str>,
    verbose: bool,
) -> Result<ssh2::Session> {
    let tcp = if let Some(port) = port {
        let port = port.parse::<u16>().or(Err(ErrorKind::InvalidPortNumber))?;
        if verbose {
            println!("Attempting to connect to {}:{}.", destination, port);
        }
        TcpStream::connect((destination, port))?
    } else {
        if verbose {
            println!("Attempting to connect to {}.", destination);
        }
        TcpStream::connect(destination).unwrap_or(TcpStream::connect((destination, 22))?)
    };
    let port = tcp.peer_addr()?.port();

    let mut session = ssh2::Session::new()?;
    session.set_timeout(10000);
    session.set_compress(true);
    session.set_tcp_stream(tcp);
    session.handshake()?;

    let session = authenticate_host(session, destination, port, verbose)?;
    let session = authenticate_session(session, username)?;

    if verbose {
        println!("Connected to host {}@{}:{}.", username, destination, port);
    }

    Ok(session)
}

/// Authenticate the identity of the host by checking the host key in `~/.ssh/known_hosts`.
fn authenticate_host(
    session: ssh2::Session,
    destination: &str,
    port: u16,
    verbose: bool,
) -> Result<ssh2::Session> {
    let mut known_hosts = session.known_hosts()?;
    let known_hosts_path = home_dir()
        .ok_or(ErrorKind::UnableToFindHomeDirectory)?
        .join(".ssh/known_hosts");
    known_hosts.read_file(&known_hosts_path, ssh2::KnownHostFileKind::OpenSSH)?;
    let (key, key_type) = session.host_key().ok_or(ErrorKind::HostKeyNotFound)?;
    match known_hosts.check_port(destination, port, key) {
        ssh2::CheckResult::Match => {
            if verbose {
                println!(
                    "Host key for {}:{} matches entry in {:?}.",
                    destination, port, known_hosts_path
                );
            }
            Ok(session)
        }
        ssh2::CheckResult::NotFound => {
            let fingerprint = session
                .host_key_hash(ssh2::HashType::Sha256)
                .map(|hash| ("SHA256", hash))
                .or_else(|| {
                    session
                        .host_key_hash(ssh2::HashType::Sha1)
                        .map(|hash| ("SHA128", hash))
                })
                .map(|(hash_type, fingerprint)| {
                    format!("{}:{}", hash_type, base64::encode(fingerprint))
                })
                .ok_or(ErrorKind::HostFingerprintNotFound)?;

            println!(
                "No matching host key for {}:{} was not found in {:?}.",
                destination, port, known_hosts_path
            );
            println!("Fingerprint: {}", fingerprint);
            print!("Would you like to add it (yes/no)? ");
            stdout().flush()?;

            let mut input = String::new();
            stdin().read_line(&mut input)?;
            match input.trim().as_ref() {
                "Y" | "y" | "YES" | "Yes" | "yes" => {
                    known_hosts.add(destination, key, "", key_type.into())?;
                    known_hosts.write_file(&known_hosts_path, ssh2::KnownHostFileKind::OpenSSH)?;
                    Ok(session)
                }
                _ => Err(ErrorKind::HostAuthenticationError(
                    destination.to_string(),
                    port,
                )),
            }
        }
        ssh2::CheckResult::Mismatch => {
            eprintln!("####################################################");
            eprintln!("# WARNING: REMOTE HOST IDENTIFICATION HAS CHANGED! #");
            eprintln!("####################################################");
            Err(ErrorKind::MismatchedFingerprint)
        }
        ssh2::CheckResult::Failure => Err(ErrorKind::HostFileCheckError),
    }
}

/// Authenticate the session using a password or public key.
fn authenticate_session(session: ssh2::Session, username: &str) -> Result<ssh2::Session> {
    let mut has_entered_password = false;

    for _ in 0..3 {
        if session.authenticated() {
            break;
        }

        let auth_methods: HashSet<&str> = session.auth_methods(username)?.split(",").collect();

        if auth_methods.contains("publickey") {
            session.userauth_agent(username).ok();
        }

        if !has_entered_password && !session.authenticated() && auth_methods.contains("password") {
            authenticate_with_password(&session, username)?;
            // We only want to prompt the user for a password for one round.
            has_entered_password = true;
        }

        // if !session.authenticated() && auth_methods.contains("keyboard-interactive") {
        //     // TODO: Need to test.
        //     struct Prompter;
        //     impl ssh2::KeyboardInteractivePrompt for Prompter {
        //         fn prompt(
        //             &mut self,
        //             _username: &str,
        //             instructions: &str,
        //             prompts: &[ssh2::Prompt],
        //         ) -> Vec<String> {
        //             prompts
        //                 .iter()
        //                 .map(|p| {
        //                     println!("{}", instructions);
        //                     if p.echo {
        //                         let mut input = String::new();
        //                         if stdin().read_line(&mut input).is_ok() {
        //                             input
        //                         } else {
        //                             String::new()
        //                         }
        //                     } else {
        //                         prompt_password_stdout(&p.text).unwrap_or_else(|_| String::new())
        //                     }
        //                 })
        //                 .collect()
        //         }
        //     }
        //     let mut prompter = Prompter;
        //     session.userauth_keyboard_interactive(username, &mut prompter)?;
        // }
    }

    if session.authenticated() {
        Ok(session)
    } else {
        Err(ErrorKind::UserAuthenticationError(username.to_string()))
    }
}

/// Attempt to authenticate the session by prompting the user for a password three times.
fn authenticate_with_password(session: &ssh2::Session, username: &str) -> Result<()> {
    for _ in 0..3 {
        let password = prompt_password_stdout("🔐 Password: ")?;
        if session.userauth_password(username, &password).is_ok() {
            return Ok(());
        } else {
            eprintln!("❌ Permission denied, please try again.");
        }
    }
    Err(ErrorKind::UserAuthenticationError(username.to_string()))
}
