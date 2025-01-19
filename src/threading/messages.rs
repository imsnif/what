/// A command sent to the update clock thread.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum ClockCmd {
    /// Pause the update clock.
    Pause,
    /// Unpause the update clock.
    Unpause,
    /// Stop the thread.
    Stop,
}

/// A command sent to the display handler thread.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum DisplayCmd {
    /// Rerender the display without updating data.
    Refresh,
    /// Consume the utilisation data generated since last update.
    ///
    /// Note that this only changes the handler state, but does not trigger a display refresh.
    Update,
    /// Cycle the order of the tables.
    ///
    /// Note that this only changes the handler state, but does not trigger a display refresh.
    CycleTables,
    /// Cleanup the display, then stop the thread.
    Stop,
}

/// A command sent to the utilisation tracker thread.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum TrackerCmd {
    /// Pause data collection.
    Pause,
    /// Unpause data collection.
    Unpause,
    /// Stop the thread.
    Stop,
}

/// A command sent to sniffer threads.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum SnifferCmd {
    /// Stop the thread.
    Stop,
}
