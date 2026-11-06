//! The entry that starts the hub at login, as the settings of the page see it: its state, on and off. Nothing registers it
//! by itself; the person does, here or with `rules-hub autostart on|off`. An entry that is there stays until they take it
//! away (an update does not touch it). It names the installed copy, whichever copy of the program is running.

use std::path::{Path, PathBuf};

use crate::background::{self, Autostart};
use appkit_hub::config::Env;

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
        if !on {
            return background::autostart(Autostart::Off).map(|_| ());
        }
        let program = background::installed_program(&Env::from_process())?;
        background::autostart_for(Autostart::On, &to_register(&program, program.is_file())?)
            .map(|_| ())
    }
}

/// The program the entry may name: the installed copy, never the one that runs. A build folder or a download goes away,
/// and the entry would then start nothing while the page still showed it as on.
fn to_register(installed: &Path, there: bool) -> Result<PathBuf, String> {
    if there {
        Ok(installed.to_path_buf())
    } else {
        Err(format!(
            "the hub is not installed yet: run `rules-hub install`, then switch it on ({} is not there)",
            installed.display()
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::to_register;
    use std::path::Path;

    #[test]
    fn the_entry_names_the_installed_copy_or_nothing() {
        let copy = Path::new("C:/apps/hub.exe");
        assert_eq!(to_register(copy, true).unwrap(), copy);
        let why = to_register(copy, false).unwrap_err();
        assert!(why.contains("install"), "{why}");
    }
}
