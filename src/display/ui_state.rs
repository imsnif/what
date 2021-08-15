use ::std::cmp;
use ::std::collections::{HashMap, HashSet, VecDeque};
use ::std::hash::Hash;
use ::std::iter::FromIterator;
use ::std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

use crate::network::{Connection, LocalSocket, Utilization};

static RECALL_LENGTH: usize = 5;
static MAX_BANDWIDTH_ITEMS: usize = 1000;

pub trait Bandwidth {
    fn get_total_bytes_downloaded(&self) -> u128;
    fn get_total_bytes_uploaded(&self) -> u128;
    fn combine_bandwidth(&mut self, other: &Self);
    fn divide_by(&mut self, amount: u128);
}

#[derive(Clone, Default)]
pub struct NetworkData {
    pub total_bytes_downloaded: u128,
    pub total_bytes_uploaded: u128,
    pub connection_count: u128,
}

#[derive(Clone, Default)]
pub struct ConnectionData {
    pub total_bytes_downloaded: u128,
    pub total_bytes_uploaded: u128,
    pub process_name: String,
    pub interface_name: String,
}

impl Bandwidth for NetworkData {
    fn get_total_bytes_downloaded(&self) -> u128 {
        self.total_bytes_downloaded
    }
    fn get_total_bytes_uploaded(&self) -> u128 {
        self.total_bytes_uploaded
    }
    fn combine_bandwidth(&mut self, other: &NetworkData) {
        self.total_bytes_downloaded += other.get_total_bytes_downloaded();
        self.total_bytes_uploaded += other.get_total_bytes_uploaded();
        self.connection_count = other.connection_count;
    }
    fn divide_by(&mut self, amount: u128) {
        self.total_bytes_downloaded /= amount;
        self.total_bytes_uploaded /= amount;
    }
}

impl Bandwidth for ConnectionData {
    fn get_total_bytes_downloaded(&self) -> u128 {
        self.total_bytes_downloaded
    }
    fn get_total_bytes_uploaded(&self) -> u128 {
        self.total_bytes_uploaded
    }
    fn combine_bandwidth(&mut self, other: &ConnectionData) {
        self.total_bytes_downloaded += other.get_total_bytes_downloaded();
        self.total_bytes_uploaded += other.get_total_bytes_uploaded();
    }
    fn divide_by(&mut self, amount: u128) {
        self.total_bytes_downloaded /= amount;
        self.total_bytes_uploaded /= amount;
    }
}

pub struct UtilizationData {
    connections_to_procs: HashMap<LocalSocket, String>,
    network_utilization: Utilization,
}

#[derive(Default)]
pub struct UIState {
    pub processes: Vec<(String, NetworkData)>,
    pub remote_addresses: Vec<(IpAddr, NetworkData)>,
    pub connections: Vec<(Connection, ConnectionData)>,
    pub total_bytes_downloaded: u128,
    pub total_bytes_uploaded: u128,
    pub cumulative_mode: bool,
    pub utilization_data: VecDeque<UtilizationData>,
    pub processes_map: HashMap<String, NetworkData>,
    pub remote_addresses_map: HashMap<IpAddr, NetworkData>,
    pub connections_map: HashMap<Connection, ConnectionData>,
}

impl UIState {
    fn get_proc_name<'a>(
        connections_to_procs: &'a HashMap<LocalSocket, String>,
        local_socket: &LocalSocket,
    ) -> Option<&'a String> {
        if let Some(process_name) = connections_to_procs.get(local_socket) {
            Some(process_name)
        } else if let Some(process_name) = connections_to_procs.get(&LocalSocket {
            ip: IpAddr::V4(Ipv4Addr::new(0, 0, 0, 0)),
            port: local_socket.port,
            protocol: local_socket.protocol,
        }) {
            Some(process_name)
        } else {
            connections_to_procs.get(&LocalSocket {
                ip: IpAddr::V6(Ipv6Addr::new(0, 0, 0, 0, 0, 0, 0, 0)),
                port: local_socket.port,
                protocol: local_socket.protocol,
            })
        }
    }
    pub fn update(
        &mut self,
        connections_to_procs: HashMap<LocalSocket, String>,
        network_utilization: Utilization,
    ) {
        self.utilization_data.push_back(UtilizationData {
            connections_to_procs,
            network_utilization,
        });
        if self.utilization_data.len() > RECALL_LENGTH {
            self.utilization_data.pop_front();
        }
        let mut processes: HashMap<String, NetworkData> = HashMap::new();
        let mut remote_addresses: HashMap<IpAddr, NetworkData> = HashMap::new();
        let mut connections: HashMap<Connection, ConnectionData> = HashMap::new();
        let mut total_bytes_downloaded: u128 = 0;
        let mut total_bytes_uploaded: u128 = 0;

        let mut seen_connections = HashSet::new();
        for state in self.utilization_data.iter().rev() {
            let connections_to_procs = &state.connections_to_procs;
            let network_utilization = &state.network_utilization;

            for (connection, connection_info) in &network_utilization.connections {
                let connection_previously_seen = !seen_connections.insert(connection);
                let connection_data = connections.entry(*connection).or_default();
                let data_for_remote_address = remote_addresses
                    .entry(connection.remote_socket.ip)
                    .or_default();
                connection_data.total_bytes_downloaded += connection_info.total_bytes_downloaded;
                connection_data.total_bytes_uploaded += connection_info.total_bytes_uploaded;
                connection_data.interface_name = connection_info.interface_name.clone();
                data_for_remote_address.total_bytes_downloaded +=
                    connection_info.total_bytes_downloaded;
                data_for_remote_address.total_bytes_uploaded +=
                    connection_info.total_bytes_uploaded;
                if !connection_previously_seen {
                    data_for_remote_address.connection_count += 1;
                }
                total_bytes_downloaded += connection_info.total_bytes_downloaded;
                total_bytes_uploaded += connection_info.total_bytes_uploaded;

                let data_for_process = if let Some(process_name) =
                    UIState::get_proc_name(connections_to_procs, &connection.local_socket)
                {
                    connection_data.process_name = process_name.clone();
                    processes.entry(process_name.clone()).or_default()
                } else {
                    connection_data.process_name = String::from("<UNKNOWN>");
                    processes
                        .entry(connection_data.process_name.clone())
                        .or_default()
                };

                data_for_process.total_bytes_downloaded += connection_info.total_bytes_downloaded;
                data_for_process.total_bytes_uploaded += connection_info.total_bytes_uploaded;
                if !connection_previously_seen {
                    data_for_process.connection_count += 1;
                }
            }
        }
        let divide_by = if self.utilization_data.is_empty() {
            1_u128
        } else {
            self.utilization_data.len() as u128
        };
        for (_, network_data) in processes.iter_mut() {
            network_data.divide_by(divide_by)
        }
        for (_, network_data) in remote_addresses.iter_mut() {
            network_data.divide_by(divide_by)
        }
        for (_, connection_data) in connections.iter_mut() {
            connection_data.divide_by(divide_by)
        }

        if self.cumulative_mode {
            merge_bandwidth(&mut self.processes_map, processes);
            merge_bandwidth(&mut self.remote_addresses_map, remote_addresses);
            merge_bandwidth(&mut self.connections_map, connections);
            self.total_bytes_downloaded += total_bytes_downloaded / divide_by;
            self.total_bytes_uploaded += total_bytes_uploaded / divide_by;
        } else {
            self.processes_map = processes;
            self.remote_addresses_map = remote_addresses;
            self.connections_map = connections;
            self.total_bytes_downloaded = total_bytes_downloaded / divide_by;
            self.total_bytes_uploaded = total_bytes_uploaded / divide_by;
        }
        self.processes = sort_and_prune(&mut self.processes_map);
        self.remote_addresses = sort_and_prune(&mut self.remote_addresses_map);
        self.connections = sort_and_prune(&mut self.connections_map);
    }
}

fn merge_bandwidth<K, V>(self_map: &mut HashMap<K, V>, other_map: HashMap<K, V>)
where
    K: Eq + Hash,
    V: Bandwidth,
{
    for (key, b_other) in other_map {
        self_map
            .entry(key)
            .and_modify(|b_self| b_self.combine_bandwidth(&b_other))
            .or_insert(b_other);
    }
}

fn sort_and_prune<K, V>(map: &mut HashMap<K, V>) -> Vec<(K, V)>
where
    K: Eq + Hash + Clone,
    V: Bandwidth + Clone,
{
    let mut bandwidth_list = Vec::from_iter(map.clone());
    bandwidth_list.sort_by_key(|(_, b)| {
        cmp::Reverse(b.get_total_bytes_downloaded() + b.get_total_bytes_uploaded())
    });

    if bandwidth_list.len() > MAX_BANDWIDTH_ITEMS {
        for (key, _) in &bandwidth_list[MAX_BANDWIDTH_ITEMS..] {
            map.remove(key);
        }
    }

    bandwidth_list
}
