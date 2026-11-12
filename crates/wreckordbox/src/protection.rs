//! `wreckordbox protection`: see and turn Library Protection, the switch
//! that refuses every change to the Library while it is on.

use clap::Subcommand;
use serde_json::json;
use wreckordbox_service::protection::{commands, Protection, ProtectionSet, SetProtection};
use wreckordbox_service::{codes, Client, ServiceError};

#[derive(Subcommand)]
pub enum ProtectionCommands {
    /// Refuse every change to the Library, from this command and in the
    /// window, until it is turned off again.
    On,
    /// Let the Library be changed, from this command and in the window.
    ///
    /// The window asks whether a backup exists before it turns protection
    /// off; this command does not.
    Off,
}

/// What to print for `wreckordbox protection [on|off]`.
pub fn protection(
    client: &mut Client,
    as_json: bool,
    command: Option<&ProtectionCommands>,
) -> Result<String, ServiceError> {
    let Some(command) = command else {
        let now: Protection = client.call(commands::GET, json!({}))?;
        return Ok(if as_json {
            json!(now).to_string()
        } else if now.protect_library {
            "Library Protection is on: nothing can change the Library.".to_owned()
        } else {
            "Library Protection is off: the Library can be changed.".to_owned()
        });
    };
    let protect_library = matches!(command, ProtectionCommands::On);
    let set: ProtectionSet =
        client.call(commands::SET, json!(SetProtection { protect_library }))?;
    if as_json {
        return Ok(json!(set).to_string());
    }
    let state = if set.protect_library { "on" } else { "off" };
    Ok(if set.previous == set.protect_library {
        format!("Library Protection was already {state}.")
    } else {
        format!("Library Protection is now {state}.")
    })
}

/// What a person is told beside an error, when there is something they can
/// do about it from here.
pub fn hint(error: &ServiceError) -> Option<&'static str> {
    (error.code == codes::LIBRARY_PROTECTED)
        .then_some("Or turn it off here: wreckordbox protection off")
}
