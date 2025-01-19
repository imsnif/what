mod error;
mod messages;

use std::{
    io,
    sync::{
        atomic::{AtomicU64, Ordering},
        mpsc::{Receiver, RecvTimeoutError, Sender, TryRecvError},
        Arc, Mutex,
    },
    thread::{self, JoinHandle},
    time::Duration,
};

use crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use log::{debug, info, trace};
use pnet::datalink::{DataLinkReceiver, NetworkInterface};
use ratatui::prelude::Backend;

use crate::{
    display::Ui,
    network::{Sniffer, Utilization},
    threading::{
        error::ThreadError,
        messages::{ClockCmd, DisplayCmd, SnifferCmd, TrackerCmd},
    },
};

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
/// Pause tristate of the application.
enum PauseState {
    /// Tracker is collecting data and the display is regularly refreshed.
    Running,
    /// Tracker is collecting data but the display is frozen.
    Frozen,
    /// Tracker has stopped collecting data and the display is frozen.
    Paused,
}

/// Start a thread that consumes terminal events and emits commands accordingly.
pub fn start_terminal_event_handler(
    events_iter: Box<dyn Iterator<Item = Event> + Send>,
    clock_cmd_tx: Sender<ClockCmd>,
    tracker_cmd_tx: Sender<TrackerCmd>,
    display_cmd_tx: Sender<DisplayCmd>,
) -> Result<JoinHandle<Result<(), ThreadError>>, io::Error> {
    let handler = move || {
        let mut pause = PauseState::Running;

        for event in events_iter {
            match event {
                Event::Resize(w, h) => {
                    info!("Terminal resize: ({w}, {h}).");
                    display_cmd_tx.send(DisplayCmd::Refresh)?;
                }

                Event::Key(KeyEvent {
                    modifiers: KeyModifiers::NONE,
                    code: KeyCode::Char(' '),
                    kind: KeyEventKind::Press,
                    ..
                }) => {
                    use PauseState as S;
                    match pause {
                        S::Running => {
                            info!("Running -> Frozen.");
                            clock_cmd_tx.send(ClockCmd::Pause)?;
                            pause = S::Frozen;
                        }
                        S::Frozen => {
                            info!("Frozen -> Paused.");
                            tracker_cmd_tx.send(TrackerCmd::Pause)?;
                            pause = S::Paused;
                        }
                        S::Paused => {
                            info!("Paused -> Running.");
                            clock_cmd_tx.send(ClockCmd::Unpause)?;
                            tracker_cmd_tx.send(TrackerCmd::Unpause)?;
                            pause = S::Running;
                        }
                    }
                }

                Event::Key(KeyEvent {
                    modifiers: KeyModifiers::NONE,
                    code: KeyCode::Tab,
                    kind: KeyEventKind::Press,
                    ..
                }) => {
                    info!("Cycle tables.");
                    display_cmd_tx.send(DisplayCmd::CycleTables)?;
                    display_cmd_tx.send(DisplayCmd::Refresh)?;
                }

                Event::Key(
                    KeyEvent {
                        modifiers: KeyModifiers::CONTROL,
                        code: KeyCode::Char('c'),
                        kind: KeyEventKind::Press,
                        ..
                    }
                    | KeyEvent {
                        modifiers: KeyModifiers::NONE,
                        code: KeyCode::Char('q'),
                        kind: KeyEventKind::Press,
                        ..
                    },
                ) => {
                    info!("Stop.");
                    clock_cmd_tx.send(ClockCmd::Stop)?;
                    display_cmd_tx.send(DisplayCmd::Stop)?;
                    tracker_cmd_tx.send(TrackerCmd::Stop)?;
                    // nothing more to do; terminate thread
                    return Ok(());
                }

                ev => {
                    trace!("Ignoring event: {ev:?}");
                }
            }
        }

        // something terrible happened to the terminal
        Err(ThreadError::TerminalEventsTerminated)
    };

    thread::Builder::new()
        .name("terminal-events-handler".into())
        .spawn(handler)
}

/// Start a thread that emits update commands at a regular interval.
pub fn start_update_clock(
    clock_cmd_rx: Receiver<ClockCmd>,
    display_cmd_tx: Sender<DisplayCmd>,
) -> Result<JoinHandle<Result<(), ThreadError>>, io::Error> {
    const UPDATE_INTERVAL: Duration = Duration::from_millis(1000);

    let handler = move || {
        let mut paused = false;

        loop {
            match clock_cmd_rx.recv_timeout(UPDATE_INTERVAL) {
                // no command received this tick
                Err(RecvTimeoutError::Timeout) if !paused => {
                    trace!("Scheduled update.");
                    display_cmd_tx.send(DisplayCmd::Update)?;
                    display_cmd_tx.send(DisplayCmd::Refresh)?;
                }
                // no command received this tick while paused
                Err(RecvTimeoutError::Timeout) => {
                    trace!("Skipping scheduled update.");
                }
                Ok(ClockCmd::Pause) => {
                    paused = true;
                    debug!("Pausing scheduled update.");
                }
                Ok(ClockCmd::Unpause) => {
                    paused = false;
                    debug!("Unpausing scheduled update.");
                    // trigger an update immediately
                    // IMPRV: is it better to only trigger a refresh but not an update?
                    display_cmd_tx.send(DisplayCmd::Update)?;
                    display_cmd_tx.send(DisplayCmd::Refresh)?;
                }
                Ok(ClockCmd::Stop) => {
                    debug!("Stopping scheduled update.");
                    break Ok(());
                }
                // command sender terminated early
                Err(RecvTimeoutError::Disconnected) => break Err(ThreadError::ClockCmdRecv),
            }
        }
    };

    thread::Builder::new()
        .name("update-clock".into())
        .spawn(handler)
}

/// Start a thread that consumes display commands and then updates the display
/// accordingly.
///
/// Note that this thread does not have any kind of automatic mechanism.
/// If you wish to rerender, you should explicitly send a `DisplayCmd::Refresh`.
pub fn start_display_handler(
    display_cmd_rx: Receiver<DisplayCmd>,
    ui: Ui<impl Backend + Send + 'static>,
    utilization_buffer: Arc<Mutex<Utilization>>,
) -> Result<JoinHandle<Result<(), ThreadError>>, io::Error> {
    let handler = move || {
        // the offset for table cycling.
        let mut table_cycle_offset = 0;

        for cmd in display_cmd_rx {
            match cmd {
                DisplayCmd::Refresh => {
                    todo!()
                }
                DisplayCmd::Update => todo!(),
                DisplayCmd::CycleTables => {
                    let modulo = ui.get_table_count();
                    table_cycle_offset = (table_cycle_offset + 1) % modulo;
                }
                DisplayCmd::Stop => {
                    use crossterm::{execute, terminal};

                    terminal::disable_raw_mode().map_err(ThreadError::TerminalStopFail)?;
                    execute!(&mut io::stdout(), terminal::LeaveAlternateScreen)
                        .map_err(ThreadError::TerminalStopFail)?;

                    return Ok(());
                }
            }
        }

        // all command senders terminated early
        Err(ThreadError::DisplayCmdRecv)
    };

    thread::Builder::new()
        .name("display-handler".into())
        .spawn(handler)
}

// IDEA: dynamically add and kill sniffer threads when interfaces change.
/// Start a thread that manages the utilisation data source.
pub fn start_utilization_tracker(
    sniffer_cmd_rx: Receiver<TrackerCmd>,
    utilization_buffer: Arc<Mutex<Utilization>>,
) -> Result<JoinHandle<Result<(), ThreadError>>, io::Error> {
    // let mut active_sniffers = vec![];

    todo!()
}

/// Start a sniffer thread for one interface.
///
/// Note that this thread has no notion of "pause". It will continuously write
/// data to its associated utilization buffer during its entire lifetime.
///
/// Pause handling (and the associated buffer resetting) is all done by
/// the utilization tracker thread.
fn start_sniffer(
    sniffer_cmd_rx: Receiver<SnifferCmd>,
    interface: NetworkInterface,
    frames_iter: Box<dyn DataLinkReceiver>,
    show_dns: bool,
    frame_counter: Arc<AtomicU64>,
    utilization_buffer: Arc<Mutex<Utilization>>,
) -> Result<JoinHandle<Result<(), ThreadError>>, io::Error> {
    let thread_name: String = format!("sniffer-{}", interface.name);

    let handler = move || {
        let interface_name = interface.name.clone();
        let mut sniffer = Sniffer::new(interface, frames_iter, show_dns);

        loop {
            match sniffer_cmd_rx.try_recv() {
                // no command received
                Err(TryRecvError::Empty) => {}
                Ok(SnifferCmd::Stop) => {
                    debug!("Stopping sniffer for {interface_name}.");
                    break Ok(());
                }
                // command sender terminated early
                Err(TryRecvError::Disconnected) => {
                    break Err(ThreadError::SnifferCmdRecv);
                }
            }

            // sleep happens here
            // note: `Sniffer` IS NOT an iterator!
            // `Sniffer::next` returning `None` does not mean there is no more data
            // therefore we cannot use while let
            if let Some(segment) = sniffer.next() {
                utilization_buffer.lock().unwrap().ingest(segment);
                frame_counter.fetch_add(1, Ordering::Release);
            }
        }
    };

    thread::Builder::new().name(thread_name).spawn(handler)
}
