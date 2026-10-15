//! Platform glue for running on any Linux distribution.
//!
//! Three things vary between distributions and all three are handled here, so
//! no caller needs to know about them:
//!
//! 1. **Loader paths.** On NixOS the X11, Wayland and GL libraries live in
//!    `/nix/store` and are not on the default search path. `LD_LIBRARY_PATH` is
//!    only read at process start, so the fix is to re-exec ourselves once with
//!    the path set. Everywhere else nothing happens.
//! 2. **Window backend.** winit prefers Wayland whenever `WAYLAND_DISPLAY` is
//!    set. That is the correct default on a healthy session, but a session
//!    missing xkbcommon or libwayland makes winit abort. If that happens we
//!    re-exec under XWayland.
//! 3. **Re-exec, not retry.** winit and eframe can only build an event loop
//!    once per process. Every fallback is therefore a fresh process, not a
//!    second call.

use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Set to an integer on the re-exec so each stage happens at most once.
const STAGE: &str = "BATNOTES_SETUP_STAGE";

/// Libraries winit and glutin resolve at runtime. Missing ones cause a hard
/// abort deep inside winit with a message that names only the first failure.
const RUNTIME_LIBS: &[&str] = &[
    "libX11.so.6",
    "libXcursor.so.1",
    "libXrandr.so.2",
    "libXi.so.6",
    "libXext.so.6",
    "libXrender.so.1",
    "libXfixes.so.3",
    "libX11-xcb.so.1",
    "libxcb.so.1",
    "libxcb-dri2.so.0",
    "libxcb-dri3.so.0",
    "libxcb-present.so.0",
    "libxcb-sync.so.1",
    "libxcb-xfixes.so.0",
    "libxkbcommon.so.0",
    "libGL.so.1",
    "libEGL.so.1",
    "libEGL_mesa.so.0",
    "libwayland-client.so.0",
    "libwayland-cursor.so.0",
    "libwayland-egl.so.1",
];

/// Upper bound on directories added to `LD_LIBRARY_PATH`.
const MAX_LIB_DIRS: usize = 16;

/// How many re-execs are allowed. One for the loader path, one for the window
/// backend. Beyond that the environment is not the problem.
const MAX_STAGES: u32 = 2;

fn stage() -> u32 {
    std::env::var(STAGE).ok().and_then(|v| v.parse().ok()).unwrap_or(0)
}

/// The value the *next* process should carry. Returning the current stage would
/// make the guard never advance and loop forever.
fn next_stage() -> String {
    (stage() + 1).to_string()
}

/// Re-exec this process with extra environment, or run `exec` directly when
/// that is impossible. Never returns on success.
fn reexec(vars: &[(&str, Option<&str>)]) -> ! {
    let exe = match std::env::current_exe() {
        Ok(p) => p,
        Err(e) => fail(format!("cannot locate our own executable: {e}")),
    };
    let mut cmd = Command::new(&exe);
    cmd.args(std::env::args_os().skip(1));
    for (k, v) in vars {
        match v {
            Some(val) => {
                cmd.env(k, val);
            }
            None => {
                cmd.env_remove(k);
            }
        }
    }
    // `exec` returns only on failure; on success this process is replaced.
    let e = cmd.exec();
    fail(format!("could not restart batnotes: {e}"));
}

fn fail(message: String) -> ! {
    eprintln!("batnotes: {message}");
    std::process::exit(1);
}

/// Stage 0: make the runtime libraries resolvable, re-executing once if needed.
pub fn prepare() {
    if stage() > 0 {
        return;
    }
    // Only Nix-style systems need this; checking for /nix/store leaves every
    // other distribution untouched.
    if !Path::new("/nix/store").exists() {
        return;
    }
    let dirs = nix_store_lib_dirs();
    if dirs.is_empty() {
        return;
    }
    let current = std::env::var("LD_LIBRARY_PATH").unwrap_or_default();
    let mut parts: Vec<String> = dirs.iter().map(|p| p.display().to_string()).collect();
    if !current.is_empty() {
        parts.push(current);
    }
    let joined = parts.join(":");
    if std::env::var_os("BATNOTES_DEBUG_PLATFORM").is_some() {
        eprintln!("batnotes: LD_LIBRARY_PATH set to {joined}");
    }
    let next = next_stage();
    reexec(&[(STAGE, Some(&next)), ("LD_LIBRARY_PATH", Some(&joined))]);
}

/// Called when the window could not be created. Retries once under XWayland.
///
/// Returns only if there is nothing left to try.
pub fn retry_on_xwayland(first_error: &str) -> ! {
    if stage() >= MAX_STAGES || std::env::var_os("WAYLAND_DISPLAY").is_none() {
        eprintln!("batnotes: {first_error}");
        std::process::exit(1);
    }
    eprintln!(
        "batnotes: {first_error}\n\
         batnotes: retrying under XWayland. Set BATNOTES_BACKEND=x11 to pin it."
    );
    let next = next_stage();
    reexec(&[(STAGE, Some(&next)), ("WAYLAND_DISPLAY", None)]);
}

/// Directories in the Nix store that hold the runtime libraries.
fn nix_store_lib_dirs() -> Vec<PathBuf> {
    let mut missing: Vec<&str> = RUNTIME_LIBS.to_vec();
    missing.sort_unstable();
    missing.dedup();
    let debug = std::env::var_os("BATNOTES_DEBUG_PLATFORM").is_some();

    let Ok(entries) = std::fs::read_dir("/nix/store") else { return Vec::new() };
    // Sort so the result does not depend on directory order.
    let mut dirs: Vec<PathBuf> = entries.flatten().map(|e| e.path()).collect();
    dirs.sort();

    let mut found: Vec<PathBuf> = Vec::new();
    for dir in dirs {
        // Safety net: an unbounded search path eventually exceeds the exec
        // argument limit and the restart fails outright.
        if found.len() >= MAX_LIB_DIRS {
            break;
        }
        let Some(name) = dir.file_name().and_then(|n| n.to_str()) else { continue };
        if !(name.contains("libx11")
            || name.contains("libxcursor")
            || name.contains("libxrandr")
            || name.contains("libxi-")
            || name.contains("libxext")
            || name.contains("libxkbcommon")
            || name.contains("libglvnd")
            || name.contains("mesa")
            || name.contains("libxcb")
            || name.contains("libxrender")
            || name.contains("libxfixes")
            || name.contains("wayland"))
        {
            continue;
        }
        let lib = dir.join("lib");
        if !lib.is_dir() {
            continue;
        }
        let Ok(rd) = std::fs::read_dir(&lib) else { continue };
        let present: Vec<String> = rd
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();

        let before = missing.len();
        missing.retain(|want| {
            // Match on the exact soname prefix. Matching a bare stem lets
            // "libxcb" be satisfied by "libxcb-errors", which loads fine and
            // then fails at link time.
            let stem = format!("{}.so", want.split(".so").next().unwrap_or(want));
            !present.iter().any(|f| f.starts_with(&stem))
        });
        // Only add a directory that actually resolved something, otherwise the
        // path grows without bound and includes hundreds of unrelated packages.
        if missing.len() < before {
            found.push(lib);
        }
        if missing.is_empty() {
            break;
        }
    }
    if debug && !missing.is_empty() {
        eprintln!("batnotes: unresolved libraries: {missing:?}");
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn library_names_are_sonames() {
        for l in RUNTIME_LIBS {
            assert!(l.starts_with("lib") && l.contains(".so"), "{l} is not a soname");
        }
    }

    #[test]
    fn nix_scan_terminates_and_is_sorted() {
        let dirs = nix_store_lib_dirs();
        let mut sorted = dirs.clone();
        sorted.sort();
        assert_eq!(dirs, sorted);
        assert!(dirs.len() <= MAX_LIB_DIRS, "search path grew to {} entries", dirs.len());
        // The path is passed to execve; an oversized one fails outright.
        let total: usize = dirs.iter().map(|d| d.display().to_string().len()).sum();
        assert!(total < 4000, "search path is {total} bytes");
    }

    #[test]
    fn stage_advances_monotonically() {
        // The guard must move forward, or the re-exec loops forever.
        assert_eq!(next_stage(), "1");
    }
}