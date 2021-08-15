use ::std::collections::{BTreeMap, HashMap};
use ::std::iter::FromIterator;
use ::unicode_width::UnicodeWidthChar;

use ::tui::backend::Backend;
use ::tui::layout::{Constraint, Rect};
use ::tui::style::{Color, Style};
use ::tui::terminal::Frame;
use ::tui::widgets::{Block, Borders, Row};

use crate::display::{Bandwidth, DisplayBandwidth, UIState};
use crate::network::{display_connection_string, display_ip_or_host};

use ::std::net::IpAddr;

fn display_upload_and_download(bandwidth: &impl Bandwidth, total: bool) -> String {
    format!(
        "{} / {}",
        DisplayBandwidth {
            bandwidth: bandwidth.get_total_bytes_uploaded() as f64,
            as_rate: !total,
        },
        DisplayBandwidth {
            bandwidth: bandwidth.get_total_bytes_downloaded() as f64,
            as_rate: !total,
        },
    )
}

pub enum ColumnCount {
    Two,
    Three,
}

impl ColumnCount {
    pub fn as_u16(&self) -> u16 {
        match &self {
            ColumnCount::Two => 2,
            ColumnCount::Three => 3,
        }
    }
}

pub struct ColumnData {
    column_count: ColumnCount,
    column_widths: Vec<u16>,
}

pub struct Table<'a> {
    title: &'a str,
    column_names: &'a [&'a str],
    rows: Vec<Vec<String>>,
    breakpoints: BTreeMap<u16, ColumnData>,
}

fn truncate_iter_to_unicode_width<Input, Collect>(iter: Input, width: usize) -> Collect
where
    Input: Iterator<Item = char>,
    Collect: FromIterator<char>,
{
    let mut chunk_width = 0;
    iter.take_while(|ch| {
        chunk_width += ch.width().unwrap_or(0);
        chunk_width <= width
    })
    .collect()
}

fn truncate_middle(row: &str, max_length: u16) -> String {
    if max_length < 6 {
        truncate_iter_to_unicode_width(row.chars(), max_length as usize)
    } else if row.len() as u16 > max_length {
        let split_point = (max_length as usize / 2) - 3;
        // why 3? 5 is the max size of the truncation text ([...] or [..]), 3 is ~5/2
        let first_slice = truncate_iter_to_unicode_width::<_, String>(row.chars(), split_point);
        let second_slice =
            truncate_iter_to_unicode_width::<_, Vec<_>>(row.chars().rev(), split_point)
                .into_iter()
                .rev()
                .collect::<String>();
        if max_length % 2 == 0 {
            format!("{}[...]{}", first_slice, second_slice)
        } else {
            format!("{}[..]{}", first_slice, second_slice)
        }
    } else {
        row.to_string()
    }
}

impl<'a> Table<'a> {
    pub fn create_connections_table(state: &UIState, ip_to_host: &HashMap<IpAddr, String>) -> Self {
        let connections_rows = state
            .connections
            .iter()
            .map(|(connection, connection_data)| {
                vec![
                    display_connection_string(
                        connection,
                        ip_to_host,
                        &connection_data.interface_name,
                    ),
                    connection_data.process_name.to_string(),
                    display_upload_and_download(connection_data, state.cumulative_mode),
                ]
            })
            .collect();
        let connections_title = "Utilization by connection";
        let connections_column_names = &["Connection", "Process", "Up / Down"];
        let mut breakpoints = BTreeMap::new();
        breakpoints.insert(
            0,
            ColumnData {
                column_count: ColumnCount::Two,
                column_widths: vec![20, 23],
            },
        );
        breakpoints.insert(
            70,
            ColumnData {
                column_count: ColumnCount::Three,
                column_widths: vec![30, 12, 23],
            },
        );
        breakpoints.insert(
            100,
            ColumnData {
                column_count: ColumnCount::Three,
                column_widths: vec![60, 12, 23],
            },
        );
        breakpoints.insert(
            140,
            ColumnData {
                column_count: ColumnCount::Three,
                column_widths: vec![100, 12, 23],
            },
        );
        Table {
            title: connections_title,
            column_names: connections_column_names,
            rows: connections_rows,
            breakpoints,
        }
    }
    pub fn create_processes_table(state: &UIState) -> Self {
        let processes_rows = state
            .processes
            .iter()
            .map(|(process_name, data_for_process)| {
                vec![
                    (*process_name).to_string(),
                    data_for_process.connection_count.to_string(),
                    display_upload_and_download(data_for_process, state.cumulative_mode),
                ]
            })
            .collect();
        let processes_title = "Utilization by process name";
        let processes_column_names = &["Process", "Connections", "Up / Down"];
        let mut breakpoints = BTreeMap::new();
        breakpoints.insert(
            0,
            ColumnData {
                column_count: ColumnCount::Two,
                column_widths: vec![12, 23],
            },
        );
        breakpoints.insert(
            50,
            ColumnData {
                column_count: ColumnCount::Three,
                column_widths: vec![12, 12, 23],
            },
        );
        breakpoints.insert(
            100,
            ColumnData {
                column_count: ColumnCount::Three,
                column_widths: vec![40, 12, 23],
            },
        );
        breakpoints.insert(
            140,
            ColumnData {
                column_count: ColumnCount::Three,
                column_widths: vec![40, 12, 23],
            },
        );
        Table {
            title: processes_title,
            column_names: processes_column_names,
            rows: processes_rows,
            breakpoints,
        }
    }
    pub fn create_remote_addresses_table(
        state: &UIState,
        ip_to_host: &HashMap<IpAddr, String>,
    ) -> Self {
        let remote_addresses_rows = state
            .remote_addresses
            .iter()
            .map(|(remote_address, data_for_remote_address)| {
                let remote_address = display_ip_or_host(*remote_address, ip_to_host);
                vec![
                    remote_address,
                    data_for_remote_address.connection_count.to_string(),
                    display_upload_and_download(data_for_remote_address, state.cumulative_mode),
                ]
            })
            .collect();
        let remote_addresses_title = "Utilization by remote address";
        let remote_addresses_column_names = &["Remote Address", "Connections", "Up / Down"];
        let mut breakpoints = BTreeMap::new();
        breakpoints.insert(
            0,
            ColumnData {
                column_count: ColumnCount::Two,
                column_widths: vec![15, 20],
            },
        );
        breakpoints.insert(
            70,
            ColumnData {
                column_count: ColumnCount::Three,
                column_widths: vec![30, 12, 23],
            },
        );
        breakpoints.insert(
            100,
            ColumnData {
                column_count: ColumnCount::Three,
                column_widths: vec![60, 12, 23],
            },
        );
        breakpoints.insert(
            140,
            ColumnData {
                column_count: ColumnCount::Three,
                column_widths: vec![100, 12, 23],
            },
        );
        Table {
            title: remote_addresses_title,
            column_names: remote_addresses_column_names,
            rows: remote_addresses_rows,
            breakpoints,
        }
    }
    pub fn render(&self, frame: &mut Frame<impl Backend>, rect: Rect) {
        let mut column_spacing: u16 = 0;
        let mut widths = &vec![];
        let mut column_count: &ColumnCount = &ColumnCount::Three;

        for (width_breakpoint, column_data) in self.breakpoints.iter() {
            if *width_breakpoint < rect.width {
                widths = &column_data.column_widths;
                column_count = &column_data.column_count;

                let total_column_width: u16 = widths.iter().sum();
                if rect.width < total_column_width - column_count.as_u16() {
                    column_spacing = 0;
                } else {
                    column_spacing = (rect.width - total_column_width) / column_count.as_u16();
                }
            }
        }

        let column_names = match column_count {
            ColumnCount::Two => {
                vec![self.column_names[0], self.column_names[2]] // always lose the middle column when needed
            }
            ColumnCount::Three => vec![
                self.column_names[0],
                self.column_names[1],
                self.column_names[2],
            ],
        };

        let rows = self.rows.iter().map(|row| match column_count {
            ColumnCount::Two => vec![
                truncate_middle(&row[0], widths[0]),
                truncate_middle(&row[2], widths[1]),
            ],
            ColumnCount::Three => vec![
                truncate_middle(&row[0], widths[0]),
                truncate_middle(&row[1], widths[1]),
                truncate_middle(&row[2], widths[2]),
            ],
        });

        let table_rows = rows.map(|row| Row::StyledData(row.into_iter(), Style::default()));
        let width_constraints: Vec<Constraint> =
            widths.iter().map(|w| Constraint::Length(*w)).collect();
        let table = ::tui::widgets::Table::new(column_names.into_iter(), table_rows)
            .block(Block::default().title(self.title).borders(Borders::ALL))
            .header_style(Style::default().fg(Color::Yellow))
            .widths(&width_constraints)
            .style(Style::default())
            .column_spacing(column_spacing);
        frame.render_widget(table, rect);
    }
}
