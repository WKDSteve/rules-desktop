//! The entry that starts the hub at login, as the settings of the page see it: its state, on and off. Nothing registers it
//! by itself; the person does, here or with `rules-hub autostart on|off`. An entry that is there stays until they take it
//! away (an update does not touch it).

use crate::background::{self, Autostart};

/// The login entry of this hub. The program has [`SystemLoginEntry`]; a test has one that holds a value.
pub trait LoginEntry: Send + Sync {
    /// Whether it is there; `None`: this system cannot tell, and the page offers no switch.
    fn is_on(&self) -> Option<bool>;

    /// Put it there, or take it away.
    ///
    /// # Errors
    ///
    /// What the system said.
    fn set(&self, on: bool) -> Result<(), String>;
}

/// The real entry (Windows: the `Run` key).
pub struct SystemLoginEntry;

impl LoginEntry for SystemLoginEntry {
    fn is_on(&self) -> Option<bool> {
        appkit_hub::background::autostart_of(&crate::IDENTITY)
    }

    fn set(&self, on: bool) -> Result<(), String> {
        background::autostart(if on { Autostart::On } else { Autostart::Off }).map(|_| ())
    }
}
