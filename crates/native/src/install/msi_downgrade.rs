//! Going back to an older version of an app installed with the Windows Installer.
//!
//! The app packages refuse to go over a newer copy (their upgrade rule stops with "A newer
//! version of … is already installed"), so the older version only goes in once the newer one is
//! gone. The person asked for this version by name; going back is never offered on its own.
//!
//! Order, and what each failure leaves behind:
//! 1. Before anything changes, both packages are downloaded and checked against their published
//!    checksums: the chosen one, and the installed version's own, to put back.
//! 2. The installed copy is removed. If that fails, nothing has changed.
//! 3. The chosen version is installed in the same folder and context. If that fails, or the
//!    permission prompt is refused, the installed version goes back in from its package.
//!
//! Only the order and the messages live here, so they are tested on every OS; the Windows
//! Installer calls are handed in.

#![cfg_attr(not(windows), allow(dead_code))]

use artcraft_launcher_core::protocol::CANCELLED;

/// The three Windows Installer steps of a downgrade, and the names the messages use.
pub(super) struct Steps<'a> {
    pub app: &'a str,
    /// The version going in, and the one installed now (put back on failure).
    pub chosen: &'a str,
    pub previous: &'a str,
    pub remove: &'a mut dyn FnMut() -> Result<(), String>,
    pub install_chosen: &'a mut dyn FnMut() -> Result<(), String>,
    pub reinstall_previous: &'a mut dyn FnMut() -> Result<(), String>,
}

/// Removes the installed copy, installs the chosen version, and puts the previous one back if
/// that install does not finish.
pub(super) fn run(steps: Steps<'_>) -> Result<(), String> {
    let Steps { app, chosen, previous, remove, install_chosen, reinstall_previous } = steps;
    remove()?;
    let why = match install_chosen() {
        Ok(()) => return Ok(()),
        Err(e) => reason(&e),
    };
    match reinstall_previous() {
        Ok(()) => Err(format!("{app} {chosen} couldn't be installed ({why}). {app} {previous} was put back.")),
        Err(again) => Err(format!(
            "{app} {chosen} couldn't be installed ({why}), and putting {previous} back failed too ({}). Install {app} again from the launcher.",
            reason(&again)
        )),
    }
}

/// A refused permission prompt reads as what happened, not as the cancel code.
fn reason(error: &str) -> String {
    if error == CANCELLED { "permission wasn't given".to_string() } else { error.to_string() }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use super::*;

    /// Runs a downgrade whose steps answer `remove`, `chosen` and `back`, and returns its result
    /// with the steps that ran, in order.
    fn downgrade(remove: Result<(), &str>, chosen: Result<(), &str>, back: Result<(), &str>) -> (Result<(), String>, Vec<&'static str>) {
        let ran = RefCell::new(Vec::new());
        let result = run(Steps {
            app: "FilmCraft",
            chosen: "0.13.0",
            previous: "0.14.0",
            remove: &mut || {
                ran.borrow_mut().push("remove");
                remove.map_err(str::to_string)
            },
            install_chosen: &mut || {
                ran.borrow_mut().push("install chosen");
                chosen.map_err(str::to_string)
            },
            reinstall_previous: &mut || {
                ran.borrow_mut().push("put back");
                back.map_err(str::to_string)
            },
        });
        (result, ran.into_inner())
    }

    #[test]
    fn the_newer_copy_goes_before_the_chosen_version_and_nothing_is_put_back_on_success() {
        let (result, ran) = downgrade(Ok(()), Ok(()), Ok(()));
        assert_eq!(result, Ok(()));
        assert_eq!(ran, ["remove", "install chosen"]);
    }

    #[test]
    fn a_failed_removal_changes_nothing_and_installs_nothing() {
        let (result, ran) = downgrade(Err("the Windows Installer stopped with error 1603"), Ok(()), Ok(()));
        assert_eq!(result, Err("the Windows Installer stopped with error 1603".to_string()));
        assert_eq!(ran, ["remove"]);
    }

    #[test]
    fn a_failed_install_puts_the_previous_version_back_and_says_so() {
        let (result, ran) = downgrade(Ok(()), Err("the Windows Installer stopped with error 1603"), Ok(()));
        assert_eq!(ran, ["remove", "install chosen", "put back"]);
        let message = result.unwrap_err();
        assert!(message.contains("FilmCraft 0.13.0 couldn't be installed"), "{message}");
        assert!(message.contains("FilmCraft 0.14.0 was put back"), "{message}");
    }

    #[test]
    fn a_refused_prompt_also_puts_the_previous_version_back() {
        let (result, ran) = downgrade(Ok(()), Err(CANCELLED), Ok(()));
        assert_eq!(ran, ["remove", "install chosen", "put back"]);
        let message = result.unwrap_err();
        assert!(message.contains("permission wasn't given"), "{message}");
        assert!(!message.contains(CANCELLED), "{message}");
    }

    #[test]
    fn when_putting_back_fails_too_the_person_is_told_how_to_recover() {
        let (result, _) = downgrade(Ok(()), Err("error 1603"), Err(CANCELLED));
        let message = result.unwrap_err();
        assert!(message.contains("putting 0.14.0 back failed too (permission wasn't given)"), "{message}");
        assert!(message.contains("Install FilmCraft again from the launcher"), "{message}");
    }
}
