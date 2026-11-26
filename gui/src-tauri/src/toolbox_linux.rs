//! Local Linux Toolbox actions and the desktop tools installed on this machine.

use std::io;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use serde_json::{json, Value};
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncReadExt, BufReader};
use tokio::process::Command;

use super::{
    append_toolbox_line, clean_toolbox_output, toolbox_output_for_history, MAX_TOOLBOX_ENTRY_CHARS,
};

const SYSTEM_PATH: &str = "/usr/bin:/bin:/usr/sbin:/sbin:/run/current-system/sw/bin";
const CHECK_TIMEOUT: Duration = Duration::from_secs(30);
const MAX_LINE_BYTES: usize = 16 * 1024;

#[derive(Clone, Copy)]
struct Program {
    name: &'static str,
    args: &'static [&'static str],
    desktop: Option<&'static str>,
}

impl Program {
    const fn cli(name: &'static str, args: &'static [&'static str]) -> Self {
        Self {
            name,
            args,
            desktop: None,
        }
    }

    const fn desktop(name: &'static str, desktop: &'static str) -> Self {
        Self {
            name,
            args: &[],
            desktop: Some(desktop),
        }
    }
}

enum Execution {
    Capture(Program),
    Open(&'static [Program]),
}

#[derive(Clone, Copy, serde::Serialize)]
#[serde(rename_all = "snake_case")]
enum Group {
    Check,
    Diagnostic,
    Advanced,
}

pub(super) struct Tool {
    id: &'static str,
    pub(super) label: &'static str,
    description: &'static str,
    group: Group,
    execution: Execution,
    requires: Option<&'static str>,
}

impl Tool {
    const fn check(
        id: &'static str,
        label: &'static str,
        description: &'static str,
        group: Group,
        program: &'static str,
        args: &'static [&'static str],
    ) -> Self {
        Self {
            id,
            label,
            description,
            group,
            execution: Execution::Capture(Program::cli(program, args)),
            requires: None,
        }
    }

    const fn open(
        id: &'static str,
        label: &'static str,
        description: &'static str,
        group: Group,
        programs: &'static [Program],
    ) -> Self {
        Self {
            id,
            label,
            description,
            group,
            execution: Execution::Open(programs),
            requires: None,
        }
    }

    const fn requiring(mut self, path: &'static str) -> Self {
        self.requires = Some(path);
        self
    }

    pub(super) fn captures_output(&self) -> bool {
        matches!(self.execution, Execution::Capture(_))
    }

    fn command(&self, desktop: &str) -> Option<Command> {
        if self.requires.is_some_and(|path| !Path::new(path).exists()) {
            return None;
        }
        let choices = match &self.execution {
            Execution::Capture(program) => std::slice::from_ref(program),
            Execution::Open(programs) => programs,
        };
        let (program, path) = choices.iter().find_map(|program| {
            let matches_desktop = program.desktop.is_none_or(|name| {
                desktop
                    .split(':')
                    .any(|entry| entry.eq_ignore_ascii_case(name))
            });
            matches_desktop
                .then(|| system_program(program.name))
                .flatten()
                .map(|path| (program, path))
        })?;
        let mut command = Command::new(path);
        command
            .args(program.args)
            .env("PATH", SYSTEM_PATH)
            .stdin(Stdio::null());
        // An AppImage's libraries belong to the bundled app. The host's
        // utilities and their children need the host's libraries and modules.
        if std::env::var_os("APPIMAGE").is_some() {
            for name in [
                "LD_LIBRARY_PATH",
                "LD_PRELOAD",
                "GTK_PATH",
                "GIO_MODULE_DIR",
            ] {
                command.env_remove(name);
            }
        }
        Some(command)
    }
}

const TOOLS: &[Tool] = &[
    Tool::check(
        "linux_disk_space",
        "Disk space",
        "Check free space on local filesystems.",
        Group::Check,
        "df",
        &["-h", "-l", "-x", "tmpfs", "-x", "devtmpfs"],
    ),
    Tool::check(
        "linux_memory",
        "Memory and swap",
        "Check how much memory and swap are in use.",
        Group::Check,
        "free",
        &["-h"],
    ),
    Tool::check(
        "linux_failed_services",
        "Failed services",
        "Find failed systemd services.",
        Group::Check,
        "systemctl",
        &[
            "list-units",
            "--type=service",
            "--state=failed",
            "--no-pager",
            "--full",
        ],
    )
    .requiring("/run/systemd/system"),
    Tool::check(
        "linux_package_audit",
        "Debian package audit",
        "Find partially installed packages and missing package metadata.",
        Group::Check,
        "dpkg",
        &["--audit"],
    )
    .requiring("/var/lib/dpkg/status"),
    Tool::check(
        "linux_flush_dns",
        "Flush DNS cache",
        "Clear cached lookups in systemd-resolved.",
        Group::Check,
        "resolvectl",
        &["flush-caches"],
    )
    .requiring("/run/systemd/resolve/resolv.conf"),
    Tool::check(
        "linux_os_release",
        "Linux distribution",
        "Show the installed distribution and version.",
        Group::Diagnostic,
        "cat",
        &["/etc/os-release"],
    )
    .requiring("/etc/os-release"),
    Tool::check(
        "linux_kernel",
        "Kernel and architecture",
        "Show the running kernel version and machine architecture.",
        Group::Diagnostic,
        "uname",
        &["-srm"],
    ),
    Tool::check(
        "linux_storage",
        "Storage devices",
        "List disks, partitions, filesystems, and mount points.",
        Group::Diagnostic,
        "lsblk",
        &["--output", "NAME,TYPE,SIZE,FSTYPE,MOUNTPOINTS"],
    ),
    Tool::check(
        "linux_interfaces",
        "Network interfaces",
        "Review interface state and IP addresses.",
        Group::Diagnostic,
        "ip",
        &["-brief", "address", "show"],
    ),
    Tool::check(
        "linux_routes",
        "IPv4 routes",
        "Review the default gateway and other IPv4 routes.",
        Group::Diagnostic,
        "ip",
        &["route", "show"],
    ),
    Tool::check(
        "linux_dns_config",
        "DNS configuration",
        "Read the configured name servers and search domains.",
        Group::Diagnostic,
        "cat",
        &["/etc/resolv.conf"],
    )
    .requiring("/etc/resolv.conf"),
    Tool::check(
        "linux_dns_status",
        "DNS resolver status",
        "Show upstream and per-interface DNS settings from systemd-resolved.",
        Group::Diagnostic,
        "resolvectl",
        &["status"],
    )
    .requiring("/run/systemd/resolve/resolv.conf"),
    Tool::check(
        "linux_system_logs",
        "Recent system warnings",
        "Read up to 100 warnings and errors from this boot that your account can access.",
        Group::Diagnostic,
        "journalctl",
        &["--boot", "--priority=warning", "--lines=100", "--no-pager"],
    ),
    Tool::check(
        "linux_processes",
        "Process snapshot",
        "Review running processes sorted by CPU use.",
        Group::Diagnostic,
        "ps",
        &["-e", "-o", "pid,ppid,stat,pcpu,pmem,comm", "--sort=-pcpu"],
    ),
    Tool::open(
        "linux_settings",
        "Desktop settings",
        "Open settings for this desktop.",
        Group::Diagnostic,
        &[
            Program::desktop("gnome-control-center", "GNOME"),
            Program::desktop("systemsettings", "KDE"),
            Program::desktop("systemsettings5", "KDE"),
            Program::desktop("xfce4-settings-manager", "XFCE"),
            Program::desktop("cinnamon-settings", "X-Cinnamon"),
            Program::desktop("cinnamon-settings", "Cinnamon"),
            Program::desktop("mate-control-center", "MATE"),
        ],
    ),
    Tool::open(
        "linux_system_monitor",
        "System Monitor",
        "Open a live view of processes and resource usage.",
        Group::Diagnostic,
        &[
            Program::desktop("gnome-system-monitor", "GNOME"),
            Program::desktop("plasma-systemmonitor", "KDE"),
            Program::desktop("ksysguard", "KDE"),
            Program::desktop("xfce4-taskmanager", "XFCE"),
            Program::desktop("mate-system-monitor", "MATE"),
            Program::cli("gnome-system-monitor", &[]),
            Program::cli("plasma-systemmonitor", &[]),
            Program::cli("xfce4-taskmanager", &[]),
        ],
    ),
    Tool::check(
        "linux_pci_devices",
        "PCI devices and drivers",
        "List PCI hardware and its kernel drivers.",
        Group::Advanced,
        "lspci",
        &["-nnk"],
    ),
    Tool::check(
        "linux_usb_devices",
        "USB devices",
        "List USB hardware detected by Linux.",
        Group::Advanced,
        "lsusb",
        &[],
    ),
    Tool::open(
        "linux_disks",
        "Disks",
        "Open GNOME Disks to inspect or manage drives and partitions.",
        Group::Advanced,
        &[Program::cli("gnome-disks", &[])],
    ),
];

fn system_program(name: &str) -> Option<PathBuf> {
    SYSTEM_PATH
        .split(':')
        .map(|dir| Path::new(dir).join(name))
        .find(|path| {
            path.metadata()
                .is_ok_and(|meta| meta.is_file() && meta.permissions().mode() & 0o111 != 0)
        })
}

fn current_desktop() -> String {
    std::env::var("XDG_CURRENT_DESKTOP")
        .or_else(|_| std::env::var("XDG_SESSION_DESKTOP"))
        .unwrap_or_default()
}

pub(super) fn catalog() -> Vec<Value> {
    let desktop = current_desktop();
    TOOLS
        .iter()
        .filter(|tool| tool.command(&desktop).is_some())
        .map(|tool| {
            let persistent = tool.captures_output();
            json!({
                "id": tool.id,
                "title": tool.label,
                "description": tool.description,
                "group": tool.group,
                "cta": if persistent { "Run" } else { "Open" },
                "persistent": persistent,
            })
        })
        .collect()
}

pub(super) fn lookup(id: &str) -> Option<&'static Tool> {
    TOOLS.iter().find(|tool| tool.id == id)
}

pub(super) async fn run(tool: &Tool, progress: impl FnMut(&str, &str)) -> Result<String, String> {
    let mut command = tool
        .command(&current_desktop())
        .ok_or_else(|| format!("{} is unavailable on this Linux installation.", tool.label))?;
    if tool.captures_output() {
        return capture(command, tool.label, progress).await;
    }

    let mut child = command
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|error| format!("opening {}: {error}", tool.label))?;
    match tokio::time::timeout(Duration::from_millis(350), child.wait()).await {
        Ok(Ok(status)) if status.success() => {}
        Ok(Ok(status)) => return Err(format!("{} exited with {status}.", tool.label)),
        Ok(Err(error)) => return Err(format!("opening {}: {error}", tool.label)),
        Err(_) => {
            tokio::spawn(async move {
                let _ = child.wait().await;
            });
        }
    }
    Ok(format!("{} opened.", tool.label))
}

async fn capture(
    mut command: Command,
    label: &str,
    mut progress: impl FnMut(&str, &str),
) -> Result<String, String> {
    let mut child = command
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(|error| format!("starting {label}: {error}"))?;
    let mut stdout = BufReader::new(child.stdout.take().expect("stdout was piped"));
    let mut stderr = BufReader::new(child.stderr.take().expect("stderr was piped"));
    let mut stdout_line = Vec::new();
    let mut stderr_line = Vec::new();
    let mut stdout_open = true;
    let mut stderr_open = true;
    let mut output = String::new();
    let result = tokio::time::timeout(CHECK_TIMEOUT, async {
        while stdout_open || stderr_open {
            let (has_line, raw, stream) = tokio::select! {
                result = read_line(&mut stdout, &mut stdout_line), if stdout_open => {
                    stdout_open = result?;
                    (stdout_open, &mut stdout_line, "stdout")
                }
                result = read_line(&mut stderr, &mut stderr_line), if stderr_open => {
                    stderr_open = result?;
                    (stderr_open, &mut stderr_line, "stderr")
                }
            };
            if has_line {
                let line = clean_toolbox_output(raw);
                raw.clear();
                append_toolbox_line(&mut output, &line);
                if output.len() > MAX_TOOLBOX_ENTRY_CHARS + 64 * 1024 {
                    output = toolbox_output_for_history(&output);
                }
                progress(stream, &line);
            }
        }
        child.wait().await
    })
    .await;

    let output = toolbox_output_for_history(output.trim());
    let error = match result {
        Ok(Ok(status)) if status.success() => {
            return Ok(if output.is_empty() {
                format!("{label} completed.")
            } else {
                output
            })
        }
        Ok(Ok(status)) => format!("{label} failed ({status})."),
        Ok(Err(error)) => format!("{label} failed: {error}"),
        Err(_) => format!(
            "{label} timed out after {} seconds.",
            CHECK_TIMEOUT.as_secs()
        ),
    };
    Err(if output.is_empty() {
        error
    } else {
        format!("{error}\n\n{output}")
    })
}

// read_until preserves partial bytes if the other pipe wins the select.
// Capping each read also keeps unusually large journal records bounded.
async fn read_line(
    reader: &mut (impl AsyncBufRead + Unpin),
    raw: &mut Vec<u8>,
) -> io::Result<bool> {
    reader
        .take((MAX_LINE_BYTES - raw.len()) as u64)
        .read_until(b'\n', raw)
        .await?;
    Ok(!raw.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn linux_actions_are_fixed_and_keep_launches_transient() {
        assert!(lookup("linux_storage").unwrap().captures_output());
        assert!(!lookup("linux_settings").unwrap().captures_output());
        assert!(lookup("sfc").is_none());
        assert!(lookup("linux_storage; sh").is_none());
    }

    #[tokio::test]
    async fn captures_both_streams_and_keeps_nonzero_status_with_output() {
        let mut command = Command::new("/bin/sh");
        command.args([
            "-c",
            "printf 'available space\\n'; printf 'device unavailable' >&2; exit 7",
        ]);
        let mut progress = Vec::new();
        let error = capture(command, "Example check", |stream, line| {
            progress.push((stream.to_string(), line.to_string()));
        })
        .await
        .unwrap_err();
        assert!(progress.contains(&("stdout".into(), "available space".into())));
        assert!(progress.contains(&("stderr".into(), "device unavailable".into())));
        assert!(
            error.contains("7") && error.contains("device unavailable"),
            "{error}"
        );
    }
}
