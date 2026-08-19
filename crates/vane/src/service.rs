use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::error::VaneCliError;

#[cfg(target_os = "macos")]
const MAC_LABEL: &str = "com.vane.daemon";
#[cfg(target_os = "macos")]
const MAC_PLIST: &str = "com.vane.daemon.plist";
#[cfg(target_os = "linux")]
const LINUX_UNIT: &str = "vane.service";

#[derive(Debug, Clone)]
pub struct ServicePaths {
    pub unit_path: PathBuf,
}

fn env_user_home() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
}

pub fn service_paths_for(user_home: &Path) -> ServicePaths {
    #[cfg(target_os = "macos")]
    {
        ServicePaths {
            unit_path: user_home
                .join("Library")
                .join("LaunchAgents")
                .join(MAC_PLIST),
        }
    }
    #[cfg(target_os = "linux")]
    {
        ServicePaths {
            unit_path: user_home
                .join(".config")
                .join("systemd")
                .join("user")
                .join(LINUX_UNIT),
        }
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        let _ = user_home;
        ServicePaths {
            unit_path: PathBuf::from("/unsupported/vane.service"),
        }
    }
}

pub fn service_paths_from_env() -> ServicePaths {
    #[cfg(target_os = "linux")]
    {
        let config = std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| env_user_home().join(".config"));
        ServicePaths {
            unit_path: config.join("systemd").join("user").join(LINUX_UNIT),
        }
    }
    #[cfg(not(target_os = "linux"))]
    {
        service_paths_for(&env_user_home())
    }
}

pub fn may_invoke_service_manager(unit_path: &Path) -> bool {
    let tmp = std::env::temp_dir();
    !unit_path.starts_with(tmp)
}

pub fn install_user_service(home: &Path, vane_bin: &Path) -> Result<(), VaneCliError> {
    install_user_service_at(&service_paths_from_env(), home, vane_bin)
}

pub fn uninstall_user_service() -> Result<(), VaneCliError> {
    uninstall_user_service_at(&service_paths_from_env())
}

pub fn install_user_service_at(
    paths: &ServicePaths,
    home: &Path,
    vane_bin: &Path,
) -> Result<(), VaneCliError> {
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        let _ = (paths, home, vane_bin);
        return Err(VaneCliError::new(
            "user service is not supported on this platform",
        ));
    }

    if let Some(parent) = paths.unit_path.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| VaneCliError::new(format!("create {}: {e}", parent.display())))?;
    }
    let body = unit_file_body(home, vane_bin)?;
    fs::write(&paths.unit_path, &body)
        .map_err(|e| VaneCliError::new(format!("write {}: {e}", paths.unit_path.display())))?;
    if may_invoke_service_manager(&paths.unit_path) {
        let _ = activate_unit(&paths.unit_path);
    }
    Ok(())
}

pub fn uninstall_user_service_at(paths: &ServicePaths) -> Result<(), VaneCliError> {
    if paths.unit_path.is_file() && may_invoke_service_manager(&paths.unit_path) {
        let _ = deactivate_unit(&paths.unit_path);
    }
    if paths.unit_path.is_file() {
        fs::remove_file(&paths.unit_path)
            .map_err(|e| VaneCliError::new(format!("remove {}: {e}", paths.unit_path.display())))?;
    }
    Ok(())
}

pub fn start_installed_service() -> Result<bool, VaneCliError> {
    let paths = service_paths_from_env();
    if !paths.unit_path.is_file() || !may_invoke_service_manager(&paths.unit_path) {
        return Ok(false);
    }
    activate_unit(&paths.unit_path)?;
    Ok(true)
}

pub fn stop_installed_service() -> Result<bool, VaneCliError> {
    let paths = service_paths_from_env();
    if !paths.unit_path.is_file() || !may_invoke_service_manager(&paths.unit_path) {
        return Ok(false);
    }
    deactivate_unit(&paths.unit_path)?;
    Ok(true)
}

fn unit_file_body(home: &Path, vane_bin: &Path) -> Result<String, VaneCliError> {
    let home_s = path_utf8(home)?;
    let bin_s = path_utf8(vane_bin)?;
    #[cfg(target_os = "macos")]
    {
        Ok(launchd_plist(bin_s, home_s))
    }
    #[cfg(target_os = "linux")]
    {
        Ok(systemd_unit(bin_s, home_s))
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        let _ = (home_s, bin_s);
        Err(VaneCliError::new(
            "user service is not supported on this platform",
        ))
    }
}

fn path_utf8(p: &Path) -> Result<&str, VaneCliError> {
    p.to_str()
        .ok_or_else(|| VaneCliError::new(format!("non-utf8 path: {}", p.display())))
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

#[cfg(target_os = "macos")]
fn launchd_plist(bin: &str, home: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
	<key>Label</key>
	<string>{label}</string>
	<key>ProgramArguments</key>
	<array>
		<string>{bin}</string>
		<string>daemon</string>
		<string>--home</string>
		<string>{home}</string>
	</array>
	<key>RunAtLoad</key>
	<true/>
	<key>KeepAlive</key>
	<true/>
</dict>
</plist>
"#,
        label = MAC_LABEL,
        bin = xml_escape(bin),
        home = xml_escape(home),
    )
}

#[cfg(target_os = "linux")]
fn systemd_unit(bin: &str, home: &str) -> String {
    format!(
        "[Unit]\nDescription=Vane local document sidecar\n\n[Service]\nExecStart=\"{bin}\" daemon --home \"{home}\"\nRestart=on-failure\n\n[Install]\nWantedBy=default.target\n"
    )
}

fn activate_unit(unit_path: &Path) -> Result<(), VaneCliError> {
    #[cfg(target_os = "macos")]
    {
        let status = Command::new("launchctl")
            .args(["load", "-w"])
            .arg(unit_path)
            .status()
            .map_err(|e| VaneCliError::new(format!("launchctl load: {e}")))?;
        if !status.success() {
            return Err(VaneCliError::new(format!(
                "launchctl load failed with {status}"
            )));
        }
        Ok(())
    }
    #[cfg(target_os = "linux")]
    {
        let _ = Command::new("systemctl")
            .args(["--user", "daemon-reload"])
            .status();
        let status = Command::new("systemctl")
            .args(["--user", "enable", "--now"])
            .arg(LINUX_UNIT)
            .status()
            .map_err(|e| VaneCliError::new(format!("systemctl enable: {e}")))?;
        if !status.success() {
            return Err(VaneCliError::new(format!(
                "systemctl enable failed with {status}"
            )));
        }
        let _ = unit_path;
        Ok(())
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        let _ = unit_path;
        Err(VaneCliError::new(
            "user service is not supported on this platform",
        ))
    }
}

fn deactivate_unit(unit_path: &Path) -> Result<(), VaneCliError> {
    #[cfg(target_os = "macos")]
    {
        let _ = Command::new("launchctl")
            .args(["unload", "-w"])
            .arg(unit_path)
            .status();
        Ok(())
    }
    #[cfg(target_os = "linux")]
    {
        let _ = Command::new("systemctl")
            .args(["--user", "disable", "--now"])
            .arg(LINUX_UNIT)
            .status();
        let _ = unit_path;
        Ok(())
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        let _ = unit_path;
        Ok(())
    }
}
