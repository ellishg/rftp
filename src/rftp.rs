use crate::file::*;
use crate::progress::Progress;
use crate::user_message::UserMessage;

use clap;
use dirs;
use ssh2;
use std::error::Error;
use std::io::{stdin, stdout, Write};
use std::iter::Iterator;
use std::net::TcpStream;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use termion::event::Key;

pub struct Rftp {
    session: ssh2::Session,
    sftp: Arc<ssh2::Sftp>,
    files: Arc<Mutex<FileList>>,
    is_alive: bool,
    progress_bars: Vec<Arc<Progress>>,
    show_hidden_files: Arc<AtomicBool>,
    user_message: Arc<UserMessage>,
}

impl Rftp {
    pub fn new() -> Result<Self, Box<dyn Error>> {
        let matches = clap::clap_app!(
            rftp =>
                (version: clap::crate_version!())
                (author: clap::crate_authors!())
                (about: clap::crate_description!())
                (@arg destination: +required)
                (@arg port: -p --port +takes_value)
                (@arg username: -u --user +takes_value +required)
        )
        .get_matches();

        let destination = matches.value_of("destination").unwrap();
        let username = matches.value_of("username").unwrap();
        let port = matches.value_of("port");
        let session = create_session(destination, username, port)?;
        let sftp = session.sftp()?;

        let show_hidden_files = false;

        let files = Arc::new(Mutex::new(FileList::new(
            &session,
            &sftp,
            show_hidden_files,
        )?));

        Ok(Rftp {
            session,
            sftp: Arc::new(sftp),
            files,
            is_alive: true,
            progress_bars: vec![],
            show_hidden_files: Arc::new(AtomicBool::new(show_hidden_files)),
            user_message: Arc::new(UserMessage::new()),
        })
    }

    pub fn tick(&mut self) -> Result<(), Box<dyn Error>> {
        self.progress_bars.retain(|p| !p.is_finished());
        Ok(())
    }

    pub fn on_event(&mut self, key: Key) -> Result<(), Box<dyn Error>> {
        match key {
            Key::Char('Q') => {
                self.is_alive = false;
            }
            Key::Char('q') => {
                if self.progress_bars.is_empty() {
                    self.is_alive = false;
                } else {
                    self.user_message.report(
                        "There are still downloads/uploads in progress. Press Q to force quit.",
                    );
                }
            }
            Key::Char('\n') => {
                let mut files = self.files.lock().unwrap();
                match files.get_selected_entry() {
                    SelectedFileEntry::Local(entry) => {
                        if entry.is_dir() {
                            files.set_local_working_path(
                                entry.path(),
                                self.show_hidden_files.load(Ordering::Relaxed),
                            )?;
                        } else {
                            drop(files);
                            self.user_message.report(&format!(
                                "Error: Cannot enter \"{}\" because it is not a directory!",
                                entry.file_name_lossy().unwrap()
                            ));
                        }
                    }
                    SelectedFileEntry::Remote(entry) => {
                        if entry.is_dir() {
                            files.set_remote_working_path(
                                entry.path(),
                                &self.sftp,
                                self.show_hidden_files.load(Ordering::Relaxed),
                            )?;
                        } else {
                            drop(files);
                            self.user_message.report(&format!(
                                "Error: Cannot enter \"{}\" because it is not a directory!",
                                entry.file_name_lossy().unwrap()
                            ));
                        }
                    }
                    SelectedFileEntry::None => {
                        drop(files);
                        self.user_message.report("No directory selected.");
                    }
                }
            }
            Key::Char(' ') => {
                let files = self.files.lock().unwrap();
                match files.get_selected_entry() {
                    SelectedFileEntry::Local(source) => {
                        if source.is_file() {
                            let dest = files
                                .get_remote_working_path()
                                .join(source.path().file_name().unwrap());
                            drop(files);
                            self.spawn_upload(source, dest);
                        } else {
                            drop(files);
                            self.user_message.report(&format!(
                                "Error: Cannot upload \"{}\" because it is a directory!",
                                source.file_name_lossy().unwrap()
                            ));
                        }
                    }
                    SelectedFileEntry::Remote(source) => {
                        if source.is_file() {
                            let dest = files
                                .get_local_working_path()
                                .join(source.path().file_name().unwrap());
                            drop(files);
                            self.spawn_download(source, dest);
                        } else {
                            drop(files);
                            self.user_message.report(&format!(
                                "Error: Cannot download \"{}\" because it is a directory!",
                                source.file_name_lossy().unwrap()
                            ));
                        }
                    }
                    SelectedFileEntry::None => {
                        drop(files);
                        self.user_message.report("No file selected.");
                    }
                }
            }
            Key::Down | Key::Char('j') => {
                self.files.lock().unwrap().next_selected();
            }
            Key::Up | Key::Char('k') => {
                self.files.lock().unwrap().prev_selected();
            }
            Key::Left | Key::Right | Key::Char('h') | Key::Char('l') => {
                self.files.lock().unwrap().toggle_selected();
            }
            _ => {}
        };
        Ok(())
    }

    /// Spawn a task to upload `source` to `dest`, then fetch the remote files again.
    fn spawn_upload(&mut self, source: LocalFileEntry, dest: PathBuf) {
        let source_filename = source.file_name_lossy().unwrap().to_string();
        let progress = {
            let title = format!("Uploading \"{}\"", source_filename);
            let len = source.len().unwrap();
            Arc::new(Progress::new(&title, len))
        };
        self.progress_bars.push(Arc::clone(&progress));

        let sftp = Arc::clone(&self.sftp);
        let user_message = Arc::clone(&self.user_message);
        let show_hidden_files = Arc::clone(&self.show_hidden_files);
        let files = Arc::clone(&self.files);

        thread::spawn(move || {
            let result = upload(source, dest, &sftp, &progress).and({
                let mut files = files.lock().unwrap();
                files.fetch_remote_files(&sftp, show_hidden_files.load(Ordering::Relaxed))
            });
            if let Err(err) = result {
                user_message.report(&format!("Error: {}!", err.to_string()));
            } else {
                user_message.report(&format!("Finished uploading \"{}\".", source_filename));
            }
        });
    }

    /// Spawn a task to download `source` to `dest`, then fetch the local files again.
    fn spawn_download(&mut self, source: RemoteFileEntry, dest: PathBuf) {
        let source_filename = source.file_name_lossy().unwrap().to_string();
        let progress = {
            let title = format!("Downloading \"{}\"", source_filename);
            let len = source.len().unwrap();
            Arc::new(Progress::new(&title, len))
        };
        self.progress_bars.push(Arc::clone(&progress));

        let sftp = Arc::clone(&self.sftp);
        let user_message = Arc::clone(&self.user_message);
        let show_hidden_files = Arc::clone(&self.show_hidden_files);
        let files = Arc::clone(&self.files);

        thread::spawn(move || {
            let result = download(source, dest, &sftp, &progress).and({
                let mut files = files.lock().unwrap();
                files.fetch_local_files(show_hidden_files.load(Ordering::Relaxed))
            });
            if let Err(err) = result {
                user_message.report(&format!("Error: {}!", err.to_string()));
            } else {
                user_message.report(&format!("Finished downloading \"{}\".", source_filename));
            }
        });
    }

    pub fn is_alive(&self) -> bool {
        self.is_alive
    }

    pub fn draw<B>(&self, mut frame: tui::terminal::Frame<B>)
    where
        B: tui::backend::Backend,
    {
        let rect = frame.size();
        let rect = self.user_message.draw(&mut frame, rect);

        let progress_bars = self.progress_bars.iter().map(|p| p.as_ref()).collect();
        let rect = Progress::draw_progress_bars(progress_bars, &mut frame, rect);

        self.files.lock().unwrap().draw(&mut frame, rect);
    }
}

impl Drop for Rftp {
    fn drop(&mut self) {
        // TODO: Shutdown tcp stream.
        self.session
            .disconnect(Some(ssh2::DisconnectCode::ByApplication), "", None)
            .unwrap();
    }
}

fn create_session(
    destination: &str,
    username: &str,
    port: Option<&str>,
) -> Result<ssh2::Session, Box<dyn Error>> {
    let tcp = if let Some(port) = port {
        let port = port
            .parse::<u16>()
            .map_err(|_| "unable to parse port number")?;
        TcpStream::connect((destination, port))?
    } else {
        TcpStream::connect(destination).unwrap_or(TcpStream::connect((destination, 22))?)
    };

    let mut session = ssh2::Session::new()?;
    session.set_timeout(10000);
    session.set_compress(true);
    session.set_tcp_stream(tcp);
    session.handshake()?;

    let mut auth_methods = session.auth_methods(username)?.split(",");
    while !session.authenticated() {
        match auth_methods.next() {
            Some("password") => {
                // TODO: I can't get this to work yet.
                // struct Prompter;
                // impl ssh2::KeyboardInteractivePrompt for Prompter {
                //     fn prompt(
                //         &mut self,
                //         username: &str,
                //         instructions: &str,
                //         prompts: &[ssh2::Prompt],
                //     ) -> Vec<String> {
                //         println!("{}", username);
                //         println!("{}", instructions);
                //         prompts.iter().map(|p| p.text.to_string()).collect()
                //     }
                // }
                // let mut prompter = Prompter;
                // session.userauth_keyboard_interactive(username, &mut prompter)?;
            }
            Some("publickey") => {
                session.userauth_agent(username)?;
            }
            Some(auth_method) => {
                // TODO: Handle more authentication methods.
                eprintln!("Unknown authentication method \"{}\".", auth_method);
            }
            None => {
                session.userauth_agent(username)?;
                if !session.authenticated() {
                    return Err(Box::from("unable to authenticate session"));
                }
            }
        }
    }

    let mut known_hosts = session.known_hosts()?;
    let known_hosts_path = dirs::home_dir()
        .ok_or("unable to find home directory")?
        .join(".ssh/known_hosts");
    known_hosts.read_file(&known_hosts_path, ssh2::KnownHostFileKind::OpenSSH)?;
    let (key, key_type) = session.host_key().ok_or("unable to get host key")?;
    match known_hosts.check(destination, key) {
        ssh2::CheckResult::Match => Ok(session),
        ssh2::CheckResult::NotFound => {
            println!(
                "The host key for {} was not found in {:?}.",
                destination, known_hosts_path
            );
            print!("Would you like to add it (yes/no)? ");
            stdout().flush()?;
            let mut input = String::new();
            stdin().read_line(&mut input)?;
            match input.trim().as_ref() {
                "YES" | "Yes" | "yes" => {
                    known_hosts.add(destination, key, "", key_type.into())?;
                    known_hosts.write_file(&known_hosts_path, ssh2::KnownHostFileKind::OpenSSH)?;
                    Ok(session)
                }
                _ => Err(Box::from(format!(
                    "the authenticity of host {} cannot be established",
                    destination
                ))),
            }
        }
        ssh2::CheckResult::Mismatch => {
            eprintln!("####################################################");
            eprintln!("# WARNING: REMOTE HOST IDENTIFICATION HAS CHANGED! #");
            eprintln!("####################################################");
            Err(Box::from("possible person in the middle attack"))
        }
        ssh2::CheckResult::Failure => Err(Box::from("failed to check known hosts")),
    }
}
