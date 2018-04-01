#![warn(unused_extern_crates)]
extern crate badtouch;
extern crate colored;
extern crate humantime;
extern crate atty;
extern crate error_chain;

use badtouch::args;
use badtouch::fsck;
use badtouch::utils;
use badtouch::pb::ProgressBar;
use badtouch::scheduler::{Scheduler, Attempt, Msg};
use badtouch::keyboard::{Keyboard, Key};

use error_chain::ChainedError;
use colored::*;
use std::thread;
use std::fs::File;
use std::time::Instant;
use std::io::prelude::*;
use badtouch::errors::{Result, ResultExt};


enum Report {
    Some(File),
    None
}

impl Report {
    pub fn open(path: Option<String>) -> Result<Report> {
        match path {
            Some(path) => Ok(Report::Some(File::create(path)?)),
            None => Ok(Report::None),
        }
    }

    pub fn write(&mut self, user: &str, password: &str, script: &str) -> Result<()> {
        match *self {
            Report::Some(ref mut f) => {
                Ok(writeln!(f, "{}:{}:{}", script, user, password)?)
            },
            Report::None => Ok(()),
        }
    }
}

macro_rules! infof {
    ($arg1:tt, $fmt:expr, $($arg:tt)*) => (
        $arg1.bold().to_string() + " " + &(format!($fmt, $($arg)*).dimmed().to_string())
    );
}

macro_rules! info {
    ($arg1:tt, $fmt:expr, $($arg:tt)*) => (
        println!("{}", infof!($arg1, $fmt, $($arg)*));
    );
}

fn setup_dictionary_attack(pool: &mut Scheduler, args: args::Dict) -> Result<usize> {
    let users = utils::load_list(&args.users).chain_err(|| "failed to load users")?;
    info!("[+]", "loaded {} users", users.len());
    let passwords = utils::load_list(&args.passwords).chain_err(|| "failed to load passwords")?;
    info!("[+]", "loaded {} passwords", passwords.len());
    let scripts = utils::load_scripts(args.scripts).chain_err(|| "failed to load scripts")?;
    info!("[+]", "loaded {} scripts", scripts.len());

    let attempts = users.len() * passwords.len() * scripts.len();
    info!("[*]", "submitting {} jobs to threadpool with {} workers", attempts, pool.max_count());

    for user in &users {
        for password in &passwords {
            for script in &scripts {
                let attempt = Attempt::new(user, password, script);
                pool.run(attempt);
            }
        }
    }

    Ok(attempts)
}

fn setup_credential_confirmation(pool: &mut Scheduler, args: args::Creds) -> Result<usize> {
    let creds = utils::load_creds(&args.creds)?;
    info!("[+]", "loaded {} credentials", creds.len());
    let scripts = utils::load_scripts(args.scripts).chain_err(|| "failed to load scripts")?;
    info!("[+]", "loaded {} scripts", scripts.len());

    let attempts = creds.len() * scripts.len();
    info!("[*]", "submitting {} jobs to threadpool with {} workers", attempts, pool.max_count());

    for cred in creds {
        // TODO: optimization if we only have once script
        for script in &scripts {
            let attempt = Attempt::bytes(&cred, script);
            pool.run(attempt);
        }
    }

    Ok(attempts)
}

fn run() -> Result<()> {
    let args = args::parse();

    if atty::isnt(atty::Stream::Stdout) {
        colored::control::SHOULD_COLORIZE.set_override(false);
    }

    let mut pool = Scheduler::new(args.workers);

    let mut report = Report::open(args.output)?;

    let attempts = match args.subcommand {
        args::SubCommand::Dict(dict) => setup_dictionary_attack(&mut pool, dict)?,
        args::SubCommand::Creds(creds) => setup_credential_confirmation(&mut pool, creds)?,
        args::SubCommand::Fsck(fsck) => return fsck::run_fsck(fsck),
    };

    let tx = pool.tx();
    thread::spawn(move || {
        let kb = Keyboard::new();
        loop {
            let key = kb.get();
            tx.send(Msg::Key(key)).expect("failed to send key");
        }
    });

    let mut pb = ProgressBar::new(attempts as u64);
    pb.print_help();
    pb.tick();

    pool.resume();
    let start = Instant::now();

    let mut valid = 0;
    let mut retries = 0;
    let mut expired = 0;
    while pool.has_work() {
        match pool.recv() {
            Msg::Key(key) => {
                match key {
                    Key::H => pb.print_help(),
                    Key::P => {
                        pb.writeln(format!("{} {}", "[*]".bold(), "pausing threads".dimmed()));
                        pool.pause();
                    },
                    Key::R => {
                        pb.writeln(format!("{} {}", "[*]".bold(), "resuming threads".dimmed()));
                        pool.resume();
                    },
                    Key::Plus => {
                        let num = pool.incr();
                        pb.writeln(format!("{} {}", "[*]".bold(), format!("increased to {} threads", num).dimmed()));
                    },
                    Key::Minus => {
                        let num = pool.decr();
                        pb.writeln(format!("{} {}", "[*]".bold(), format!("decreased to {} threads", num).dimmed()));
                    },
                }
                pb.tick();
            },
            Msg::Attempt(mut attempt, result) => {
                match result {
                    Ok(is_valid) => {
                        if is_valid {
                            let user = attempt.user();
                            let password = attempt.password();
                            let script = attempt.script.descr();

                            pb.writeln(format!("{} {}({}) => {:?}:{:?}", "[+]".bold(), "valid".green(),
                                script.yellow(), user, password));
                            report.write(user, password, script)?;
                            valid += 1;
                        }
                        pb.inc();
                    },
                    Err(err) => {
                        pb.writeln(format!("{} {}({}, {}): {:?}", "[!]".bold(), "error".red(), attempt.script.descr().yellow(), format!("{:?}:{:?}", attempt.user(), attempt.password()).dimmed(), err));

                        if attempt.ttl > 0 {
                            // we have retries left
                            retries += 1;
                            attempt.ttl -= 1;
                            pool.run(attempt);
                            pb.tick();
                        } else {
                            // giving up
                            expired += 1;
                            pb.inc();
                        }
                    }
                };
            },
        }
    }

    let elapsed = start.elapsed();
    let average = elapsed / attempts as u32;
    pb.finish_replace(infof!("[+]", "found {} valid credentials with {} attempts and {} retries after {} and on average {} per attempt. {} attempts expired.\n",
            valid, attempts, retries,
            humantime::format_duration(elapsed),
            humantime::format_duration(average),
            expired,
    ));

    Keyboard::reset();

    Ok(())
}

fn main() {
    if let Err(ref e) = run() {
        eprintln!("{}", e.display_chain());
        std::process::exit(1);
    }
}
