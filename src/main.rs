#![deny(clippy::enum_glob_use)]

mod cli;
mod display;
mod network;
mod os;
#[cfg(test)]
mod tests;
mod threading;

use std::{
    collections::HashMap,
    fs::File,
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc, Mutex, RwLock,
    },
    thread::{self, park_timeout},
    time::{Duration, Instant},
};

use clap::Parser;
use crossterm::{
    event::{Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers},
    terminal,
};
use display::{elapsed_time, RawTerminalBackend, Ui};
use eyre::bail;
use network::{
    dns::{self, IpTable},
    LocalSocket, Sniffer, Utilization,
};
use pnet::datalink::{DataLinkReceiver, NetworkInterface};
use ratatui::backend::{Backend, CrosstermBackend};
use simplelog::WriteLogger;

use crate::cli::Opt;
use crate::os::ProcessInfo;

const UPDATE_RATE: Duration = Duration::from_millis(1000);

fn main() -> eyre::Result<()> {
    let opts = Opt::parse();

    // init logging
    if let Some(ref log_path) = opts.log_to {
        let log_file = File::options()
            .write(true)
            .create_new(true)
            .open(log_path)?;
        WriteLogger::init(
            opts.verbosity.log_level_filter(),
            Default::default(),
            log_file,
        )?;
    }

    let os_input = os::get_input(opts.interface.as_deref(), !opts.no_resolve, opts.dns_server)?;
    if opts.raw {
        let terminal_backend = RawTerminalBackend {};
        start(terminal_backend, os_input, opts);
    } else {
        let Ok(()) = terminal::enable_raw_mode() else {
            bail!(
                "Failed to get stdout: if you are trying to pipe 'bandwhich' you should use the --raw flag"
            )
        };

        let mut stdout = std::io::stdout();
        // Ignore enteralternatescreen error
        let _ = crossterm::execute!(&mut stdout, terminal::EnterAlternateScreen);
        let terminal_backend = CrosstermBackend::new(stdout);
        start(terminal_backend, os_input, opts);
    }
    Ok(())
}

pub struct OpenSockets {
    sockets_to_procs: HashMap<LocalSocket, ProcessInfo>,
}

pub struct OsInputOutput {
    pub interfaces_with_frames: Vec<(NetworkInterface, Box<dyn DataLinkReceiver>)>,
    pub get_open_sockets: fn() -> OpenSockets,
    pub terminal_events: Box<dyn Iterator<Item = Event> + Send>,
    pub dns_client: Option<dns::Client>,
    pub write_to_stdout: Box<dyn FnMut(&str) + Send>,
}

pub fn start<B>(terminal_backend: B, os_input: OsInputOutput, opts: Opt)
where
    B: Backend + Send + 'static,
{
    let running = Arc::new(AtomicBool::new(true));
    let paused = Arc::new(AtomicBool::new(false));
    let last_start_time = Arc::new(RwLock::new(Instant::now()));
    let cumulative_time = Arc::new(RwLock::new(Duration::new(0, 0)));
    let table_cycle_offset = Arc::new(AtomicUsize::new(0));

    let mut active_threads = vec![];

    let terminal_events = os_input.terminal_events;
    let get_open_sockets = os_input.get_open_sockets;
    let mut write_to_stdout = os_input.write_to_stdout;
    let mut dns_client = os_input.dns_client;

    let raw_mode = opts.raw;

    let network_utilization = Arc::new(Mutex::new(Utilization::new()));
    let ui = Arc::new(Mutex::new(Ui::new(terminal_backend, &opts)));

    let display_handler = thread::Builder::new()
        .name("display_handler".to_string())
        .spawn({
            let running = running.clone();
            let paused = paused.clone();
            let table_cycle_offset = table_cycle_offset.clone();

            let network_utilization = network_utilization.clone();
            let last_start_time = last_start_time.clone();
            let cumulative_time = cumulative_time.clone();
            let ui = ui.clone();

            move || {
                while running.load(Ordering::Acquire) {
                    let render_start_time = Instant::now();
                    let utilization = network_utilization.lock().unwrap().clone_and_reset();
                    let OpenSockets { sockets_to_procs } = get_open_sockets();
                    let mut ip_to_host = IpTable::new();
                    if let Some(dns_client) = dns_client.as_mut() {
                        ip_to_host = dns_client.cache();
                        let unresolved_ips = utilization
                            .connections
                            .keys()
                            .filter(|conn| !ip_to_host.contains_key(&conn.remote_socket.ip))
                            .map(|conn| conn.remote_socket.ip)
                            .collect::<Vec<_>>();
                        dns_client.resolve(unresolved_ips);
                    }
                    {
                        let mut ui = ui.lock().unwrap();
                        let paused = paused.load(Ordering::SeqCst);
                        let table_cycle_offset = table_cycle_offset.load(Ordering::SeqCst);
                        if !paused {
                            ui.update_state(sockets_to_procs, utilization, ip_to_host);
                        }
                        let elapsed_time = elapsed_time(
                            *last_start_time.read().unwrap(),
                            *cumulative_time.read().unwrap(),
                            paused,
                        );

                        if raw_mode {
                            ui.output_text(&mut write_to_stdout);
                        } else {
                            ui.draw(paused, elapsed_time, table_cycle_offset);
                        }
                    }
                    let render_duration = render_start_time.elapsed();
                    if render_duration < UPDATE_RATE {
                        park_timeout(UPDATE_RATE - render_duration);
                    }
                }
                if !raw_mode {
                    let mut ui = ui.lock().unwrap();
                    ui.end();
                }
            }
        })
        .unwrap();

    let terminal_event_handler = thread::Builder::new()
        .name("terminal_events_handler".to_string())
        .spawn({
            let running = running.clone();
            let display_handler = display_handler.thread().clone();

            move || {
                for evt in terminal_events {
                    let mut ui = ui.lock().unwrap();

                    match evt {
                        Event::Resize(_x, _y) if !raw_mode => {
                            let paused = paused.load(Ordering::SeqCst);
                            ui.draw(
                                paused,
                                elapsed_time(
                                    *last_start_time.read().unwrap(),
                                    *cumulative_time.read().unwrap(),
                                    paused,
                                ),
                                table_cycle_offset.load(Ordering::SeqCst),
                            );
                        }
                        Event::Key(KeyEvent {
                            modifiers: KeyModifiers::CONTROL,
                            code: KeyCode::Char('c'),
                            kind: KeyEventKind::Press,
                            ..
                        })
                        | Event::Key(KeyEvent {
                            modifiers: KeyModifiers::NONE,
                            code: KeyCode::Char('q'),
                            kind: KeyEventKind::Press,
                            ..
                        }) => {
                            running.store(false, Ordering::Release);
                            display_handler.unpark();
                            match terminal::disable_raw_mode() {
                                Ok(_) => {}
                                Err(_) => println!("Error could not disable raw input"),
                            }
                            let mut stdout = std::io::stdout();
                            if crossterm::execute!(&mut stdout, terminal::LeaveAlternateScreen)
                                .is_err()
                            {
                                println!("Error could not leave alternte screen");
                            };
                            break;
                        }
                        Event::Key(KeyEvent {
                            modifiers: KeyModifiers::NONE,
                            code: KeyCode::Char(' '),
                            kind: KeyEventKind::Press,
                            ..
                        }) => {
                            let restarting = paused.fetch_xor(true, Ordering::SeqCst);
                            if restarting {
                                *last_start_time.write().unwrap() = Instant::now();
                            } else {
                                let last_start_time_copy = *last_start_time.read().unwrap();
                                let current_cumulative_time_copy = *cumulative_time.read().unwrap();
                                let new_cumulative_time =
                                    current_cumulative_time_copy + last_start_time_copy.elapsed();
                                *cumulative_time.write().unwrap() = new_cumulative_time;
                            }

                            display_handler.unpark();
                        }
                        Event::Key(KeyEvent {
                            modifiers: KeyModifiers::NONE,
                            code: KeyCode::Tab,
                            kind: KeyEventKind::Press,
                            ..
                        }) => {
                            let paused = paused.load(Ordering::SeqCst);
                            let elapsed_time = elapsed_time(
                                *last_start_time.read().unwrap(),
                                *cumulative_time.read().unwrap(),
                                paused,
                            );
                            let table_count = ui.get_table_count();
                            let new = table_cycle_offset.load(Ordering::SeqCst) + 1 % table_count;
                            table_cycle_offset.store(new, Ordering::SeqCst);
                            ui.draw(paused, elapsed_time, new);
                        }
                        _ => (),
                    };
                }
            }
        })
        .unwrap();

    active_threads.push(display_handler);
    active_threads.push(terminal_event_handler);

    let sniffer_threads = os_input
        .interfaces_with_frames
        .into_iter()
        .map(|(iface, frames)| {
            let name = format!("sniffing_handler_{}", iface.name);
            let running = running.clone();
            let show_dns = opts.show_dns;
            let network_utilization = network_utilization.clone();

            thread::Builder::new()
                .name(name)
                .spawn(move || {
                    let mut sniffer = Sniffer::new(iface, frames, show_dns);

                    while running.load(Ordering::Acquire) {
                        if let Some(segment) = sniffer.next() {
                            network_utilization.lock().unwrap().ingest(segment);
                        }
                    }
                })
                .unwrap()
        })
        .collect::<Vec<_>>();
    active_threads.extend(sniffer_threads);

    for thread_handler in active_threads {
        thread_handler.join().unwrap()
    }
}
