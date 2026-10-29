//! Going back to an older version of an app installed with the Windows Installer.
//!
//! The app packages refuse to go over a newer copy (their upgrade rule stops with "A newer
//! version of … is already installed"), so the older version only goes in once the newer one is
//! gone. The person asked for this version by name; going back is never offered on its own.
//!
//! Before anything changes, both packages are downloaded and checked against their published
//! checksums: the chosen one, and the installed version's own, to put back. Then:
//! 1. The installed copy is removed. If that fails, nothing has changed.
//! 2. The chosen version is installed in the same folder and context. If that fails, the
//!    installed version goes back in from its package.
//!
//! An all-users copy needs Windows' permission, and the three steps run under ONE prompt, in one
//! elevated session ([`script`]). Saying no changes nothing, and once it is yes, putting the old
//! version back needs no second answer. Separate prompts proved fragile: they look identical, so
//! a refused install was followed by a refused put-back, read as a retry, leaving no app at all.
//! The session is given only `msiexec` command lines for the two held, checked packages and the
//! installed copy's product codes, as [`msi_batch`](super::msi_batch) does for Update all.
//!
//! A per-user copy needs no permission, so its steps run one after another ([`run`]); that is
//! also the fallback when the elevated session cannot start.

#![cfg_attr(not(windows), allow(dead_code))]

use artcraft_launcher_core::protocol::CANCELLED;

use super::msi_batch::{self, DECLINED, LAUNCH_FAILED};

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
/// that install does not finish, one step after another.
pub(super) fn run(steps: Steps<'_>) -> Result<(), String> {
    let Steps { app, chosen, previous, remove, install_chosen, reinstall_previous } = steps;
    remove()?;
    let why = match install_chosen() {
        Ok(()) => return Ok(()),
        Err(e) => reason(&e),
    };
    match reinstall_previous() {
        Ok(()) => Err(put_back(app, chosen, previous, &why)),
        Err(again) => Err(not_put_back(app, chosen, previous, &why, &reason(&again))),
    }
}

fn put_back(app: &str, chosen: &str, previous: &str, why: &str) -> String {
    format!("{app} {chosen} couldn't be installed ({why}). {app} {previous} was put back.")
}

fn not_put_back(app: &str, chosen: &str, previous: &str, why: &str, again: &str) -> String {
    format!("{app} {chosen} couldn't be installed ({why}), and putting {previous} back failed too ({again}). Install {app} again from the launcher.")
}

/// A refused permission prompt reads as what happened, not as the cancel code.
fn reason(error: &str) -> String {
    if error == CANCELLED { "permission wasn't given".to_string() } else { error.to_string() }
}

/// The step that stopped the elevated session, in its exit code: bit 29 (the "customer" bit, which
/// no Windows or Windows Installer code sets), the step in bits 24-27, and the `msiexec` codes in
/// bits 12-23 and 0-11. Windows Installer codes are below 4096.
const OURS: i32 = 0x2000_0000;
const REMOVAL_FAILED: i32 = 1;
const PUT_BACK: i32 = 2;
const NOT_PUT_BACK: i32 = 3;

/// The elevated session: removes each installed registration (`remove`, `msiexec` arguments),
/// installs `chosen`, and on failure installs `previous`, all through `program`. It exits 0 when
/// the chosen version went in, or with one of the codes [`ended`] reads.
pub(super) fn script(program: &str, remove: &[String], chosen: &str, previous: &str) -> String {
    let list = remove.iter().map(|l| msi_batch::quoted(l)).collect::<Vec<_>>().join(",");
    format!(
        "$ErrorActionPreference = 'Stop'\n\
         $ProgressPreference = 'SilentlyContinue'\n\
         $program = {program}\n\
         function Run([string]$arguments) {{ (Start-Process -FilePath $program -ArgumentList $arguments -Wait -PassThru).ExitCode }}\n\
         function Fine([int]$code) {{ $code -eq 0 -or $code -eq 3010 -or $code -eq 1641 }}\n\
         function Code([int]$code) {{ if ($code -lt 0 -or $code -gt 4094) {{ 4095 }} else {{ $code }} }}\n\
         function Ended([int]$step, [int]$first, [int]$second) {{ {OURS} -bor ($step -shl 24) -bor ((Code $first) -shl 12) -bor (Code $second) }}\n\
         foreach ($arguments in @({list})) {{\n\
         \x20 $code = Run $arguments\n\
         \x20 if (-not (Fine $code)) {{ exit (Ended {REMOVAL_FAILED} $code 0) }}\n\
         }}\n\
         $code = Run {chosen}\n\
         if (Fine $code) {{ exit 0 }}\n\
         $back = Run {previous}\n\
         if (Fine $back) {{ exit (Ended {PUT_BACK} $code 0) }}\n\
         exit (Ended {NOT_PUT_BACK} $code $back)\n",
        program = msi_batch::quoted(program),
        chosen = msi_batch::quoted(chosen),
        previous = msi_batch::quoted(previous),
    )
}

/// What the elevated session's exit code says happened.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum Ended {
    Done,
    /// The one prompt was refused: nothing ran.
    Declined,
    /// The session never started; the per-user route can still be tried.
    LaunchFailed,
    RemovalFailed(i32),
    /// The chosen version failed with this code, and the previous one is back.
    PutBack(i32),
    NotPutBack {
        install: i32,
        back: i32,
    },
    Other(Option<i32>),
}

pub(super) fn ended(code: Option<i32>) -> Ended {
    let Some(code) = code else { return Ended::Other(None) };
    let step = (code >> 24) & 0xF;
    let (first, second) = ((code >> 12) & 0xFFF, code & 0xFFF);
    match code {
        0 => Ended::Done,
        DECLINED => Ended::Declined,
        LAUNCH_FAILED => Ended::LaunchFailed,
        _ if code & !0x0FFF_FFFF == OURS && step == REMOVAL_FAILED => Ended::RemovalFailed(first),
        _ if code & !0x0FFF_FFFF == OURS && step == PUT_BACK => Ended::PutBack(first),
        _ if code & !0x0FFF_FFFF == OURS && step == NOT_PUT_BACK => Ended::NotPutBack { install: first, back: second },
        _ => Ended::Other(Some(code)),
    }
}

/// What a finished elevated downgrade means for the person. `LaunchFailed` is not passed here:
/// the caller falls back to [`run`] instead.
pub(super) fn report(ended: Ended, app: &str, chosen: &str, previous: &str) -> Result<(), String> {
    // Inside the session permission was already given: a cancel there is the progress window's.
    let msiexec = |code: i32| match super::explain(Some(code)).err().unwrap_or_default() {
        e if e == CANCELLED => "it was cancelled".to_string(),
        e => e,
    };
    match ended {
        Ended::Done => Ok(()),
        Ended::Declined => Err(CANCELLED.to_string()),
        Ended::RemovalFailed(code) => Err(format!("{app} {previous} couldn't be removed ({}), so nothing was changed.", msiexec(code))),
        Ended::PutBack(code) => Err(put_back(app, chosen, previous, &msiexec(code))),
        Ended::NotPutBack { install, back } => Err(not_put_back(app, chosen, previous, &msiexec(install), &msiexec(back))),
        Ended::LaunchFailed | Ended::Other(_) => {
            let code = match ended {
                Ended::Other(Some(code)) => code.to_string(),
                _ => "none".to_string(),
            };
            Err(format!("going back to {app} {chosen} stopped unexpectedly (code {code}). Check which version of {app} is installed."))
        }
    }
}

/// The `msiexec` arguments that remove one registration. Product codes come from the registry, so
/// only the exact `{GUID}` shape is let into the elevated session.
pub(super) fn removal(product_code: &str) -> Result<String, String> {
    let inner = product_code.strip_prefix('{').and_then(|c| c.strip_suffix('}')).unwrap_or("");
    let shape = inner.len() == 36 && inner.char_indices().all(|(i, c)| if matches!(i, 8 | 13 | 18 | 23) { c == '-' } else { c.is_ascii_hexdigit() });
    if !shape {
        return Err(format!("couldn't read the installed copy's registration ({product_code})"));
    }
    Ok(format!("/x {product_code} /passive /norestart"))
}

#[cfg(test)]
#[path = "msi_downgrade_tests.rs"]
mod tests;
