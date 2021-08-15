use ::crossterm::event::read;
use ::crossterm::event::Event;
use ::pnet::datalink::Channel::Ethernet;
use ::pnet::datalink::DataLinkReceiver;
use ::pnet::datalink::{self, Config, NetworkInterface};
use ::std::io::{self, ErrorKind, Write};
use ::tokio::runtime::Runtime;

use ::std::net::Ipv4Addr;
use ::std::time;

use crate::os::errors::GetInterfaceErrorKind;

#[cfg(target_os = "linux")]
use crate::os::linux::get_open_sockets;
#[cfg(any(target_os = "macos", target_os = "freebsd"))]
use crate::os::lsof::get_open_sockets;
#[cfg(target_os = "windows")]
use crate::os::windows::get_open_sockets;

use crate::{network::dns, OsInputOutput};

pub struct TerminalEvents;

impl Iterator for TerminalEvents {
    type Item = Event;
    fn next(&mut self) -> Option<Event> {
        match read() {
            Ok(ev) => Some(ev),
            Err(_) => None,
        }
    }
}

pub(crate) fn get_datalink_channel(
    interface: &NetworkInterface,
) -> Result<Box<dyn DataLinkReceiver>, GetInterfaceErrorKind> {
    let config = Config {
        read_timeout: Some(time::Duration::new(1, 0)),
        read_buffer_size: 65536,
        ..Default::default()
    };

    match datalink::channel(interface, config) {
        Ok(Ethernet(_tx, rx)) => Ok(rx),
        Ok(_) => Err(GetInterfaceErrorKind::OtherError(format!(
            "{}: Unsupported interface type",
            interface.name
        ))),
        Err(e) => match e.kind() {
            ErrorKind::PermissionDenied => Err(GetInterfaceErrorKind::PermissionError(
                interface.name.to_owned(),
            )),
            _ => Err(GetInterfaceErrorKind::OtherError(format!(
                "{}: {}",
                &interface.name, e
            ))),
        },
    }
}

fn get_interface(interface_name: &str) -> Option<NetworkInterface> {
    datalink::interfaces()
        .into_iter()
        .find(|iface| iface.name == interface_name)
}

fn create_write_to_stdout() -> Box<dyn FnMut(String) + Send> {
    Box::new({
        let mut stdout = io::stdout();
        move |output: String| {
            writeln!(stdout, "{}", output).unwrap();
        }
    })
}

#[derive(Debug)]
pub struct UserErrors {
    permission: Option<String>,
    other: Option<String>,
}

pub fn collect_errors<'a, I>(network_frames: I) -> String
where
    I: Iterator<
        Item = (
            &'a NetworkInterface,
            Result<Box<dyn DataLinkReceiver>, GetInterfaceErrorKind>,
        ),
    >,
{
    let errors = network_frames.fold(
        UserErrors {
            permission: None,
            other: None,
        },
        |acc, (_, elem)| {
            if let Some(iface_error) = elem.err() {
                match iface_error {
                    GetInterfaceErrorKind::PermissionError(interface_name) => {
                        if let Some(prev_interface) = acc.permission {
                            return UserErrors {
                                permission: Some(format!("{}, {}", prev_interface, interface_name)),
                                ..acc
                            };
                        } else {
                            return UserErrors {
                                permission: Some(interface_name),
                                ..acc
                            };
                        }
                    }
                    error => {
                        if let Some(prev_errors) = acc.other {
                            return UserErrors {
                                other: Some(format!("{} \n {}", prev_errors, error)),
                                ..acc
                            };
                        } else {
                            return UserErrors {
                                other: Some(format!("{}", error)),
                                ..acc
                            };
                        }
                    }
                };
            }
            acc
        },
    );
    if let Some(interface_name) = errors.permission {
        if let Some(other_errors) = errors.other {
            format!(
                "\n\n{}: {} \nAdditional Errors: \n {}",
                interface_name,
                eperm_message(),
                other_errors
            )
        } else {
            format!("\n\n{}: {}", interface_name, eperm_message())
        }
    } else {
        let other_errors = errors
            .other
            .expect("asked to collect errors but found no errors");
        format!("\n\n {}", other_errors)
    }
}

pub fn get_input(
    interface_name: &Option<String>,
    resolve: bool,
    dns_server: &Option<Ipv4Addr>,
) -> Result<OsInputOutput, failure::Error> {
    let network_interfaces = if let Some(name) = interface_name {
        match get_interface(name) {
            Some(interface) => vec![interface],
            None => {
                failure::bail!("Cannot find interface {}", name);
                // the homebrew formula relies on this wording, please be careful when changing
            }
        }
    } else {
        datalink::interfaces()
    };

    #[cfg(any(target_os = "windows"))]
    let network_frames = network_interfaces
        .iter()
        .filter(|iface| !iface.ips.is_empty())
        .map(|iface| (iface, get_datalink_channel(iface)));
    #[cfg(not(target_os = "windows"))]
    let network_frames = network_interfaces
        .iter()
        .filter(|iface| iface.is_up() && !iface.ips.is_empty())
        .map(|iface| (iface, get_datalink_channel(iface)));

    let (available_network_frames, network_interfaces) = {
        let network_frames = network_frames.clone();
        let mut available_network_frames = Vec::new();
        let mut available_interfaces: Vec<NetworkInterface> = Vec::new();
        for (iface, rx) in network_frames.filter_map(|(iface, channel)| {
            if let Ok(rx) = channel {
                Some((iface, rx))
            } else {
                None
            }
        }) {
            available_interfaces.push(iface.clone());
            available_network_frames.push(rx);
        }
        (available_network_frames, available_interfaces)
    };

    if available_network_frames.is_empty() {
        let all_errors = collect_errors(network_frames.clone());
        if !all_errors.is_empty() {
            failure::bail!(all_errors);
        }

        failure::bail!("Failed to find any network interface to listen on.");
    }

    let keyboard_events = Box::new(TerminalEvents);
    let write_to_stdout = create_write_to_stdout();
    let dns_client = if resolve {
        let mut runtime = Runtime::new()?;
        let resolver =
            match runtime.block_on(dns::Resolver::new(runtime.handle().clone(), dns_server)) {
                Ok(resolver) => resolver,
                Err(err) => failure::bail!(
                    "Could not initialize the DNS resolver. Are you offline?\n\nReason: {:?}",
                    err
                ),
            };
        let dns_client = dns::Client::new(resolver, runtime)?;
        Some(dns_client)
    } else {
        None
    };

    Ok(OsInputOutput {
        network_interfaces,
        network_frames: available_network_frames,
        get_open_sockets,
        terminal_events: keyboard_events,
        dns_client,
        write_to_stdout,
    })
}

#[inline]
#[cfg(any(target_os = "macos", target_os = "freebsd"))]
fn eperm_message() -> &'static str {
    "Insufficient permissions to listen on network interface(s). Try running with sudo."
}

#[inline]
#[cfg(target_os = "linux")]
fn eperm_message() -> &'static str {
    r#"
    Insufficient permissions to listen on network interface(s). You can work around
    this issue like this:

    * Try running `bandwhich` with `sudo`

    * Build a `setcap(8)` wrapper for `bandwhich` with the following rules:
        `cap_sys_ptrace,cap_dac_read_search,cap_net_raw,cap_net_admin+ep`
    "#
}

#[inline]
#[cfg(any(target_os = "windows"))]
fn eperm_message() -> &'static str {
    "Insufficient permissions to listen on network interface(s). Try running with administrator rights."
}
