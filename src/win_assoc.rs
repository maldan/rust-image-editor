//! Register / unregister `.mgimg` file association (per-user HKCU).

#[cfg(windows)]
pub fn register() -> Result<(), String> {
    use std::env;
    use winreg::enums::*;
    use winreg::RegKey;

    let exe_path = env::current_exe().map_err(|e| e.to_string())?;
    let exe = exe_path.to_string_lossy();
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let (classes, _) = hkcu
        .create_subkey("Software\\Classes")
        .map_err(|e| e.to_string())?;

    let prog_id = "MegaImageEditor.mgimg";
    {
        let (key, _) = classes.create_subkey(prog_id).map_err(|e| e.to_string())?;
        key.set_value("", &"Mega Image Project")
            .map_err(|e| e.to_string())?;

        let (icon, _) = key
            .create_subkey("DefaultIcon")
            .map_err(|e| e.to_string())?;
        // Index 0 = first icon embedded in the exe (via winresource).
        icon.set_value("", &format!("\"{exe}\",0"))
            .map_err(|e| e.to_string())?;

        let (cmd, _) = key
            .create_subkey("shell\\open\\command")
            .map_err(|e| e.to_string())?;
        cmd.set_value("", &format!("\"{exe}\" \"%1\""))
            .map_err(|e| e.to_string())?;
    }
    {
        let (ext, _) = classes.create_subkey(".mgimg").map_err(|e| e.to_string())?;
        ext.set_value("", &prog_id).map_err(|e| e.to_string())?;
        // Some shells also look at extension-level DefaultIcon.
        let (icon, _) = ext
            .create_subkey("DefaultIcon")
            .map_err(|e| e.to_string())?;
        icon.set_value("", &format!("\"{exe}\",0"))
            .map_err(|e| e.to_string())?;
    }

    notify_shell();
    Ok(())
}

#[cfg(windows)]
pub fn unregister() -> Result<(), String> {
    use winreg::enums::*;
    use winreg::RegKey;

    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let classes = hkcu
        .open_subkey_with_flags("Software\\Classes", KEY_WRITE)
        .map_err(|e| e.to_string())?;
    let _ = classes.delete_subkey_all("MegaImageEditor.mgimg");
    if let Ok(ext) = classes.open_subkey_with_flags(".mgimg", KEY_READ | KEY_WRITE) {
        let val: Result<String, _> = ext.get_value("");
        if matches!(val.as_deref(), Ok("MegaImageEditor.mgimg")) {
            let _ = classes.delete_subkey_all(".mgimg");
        }
    }
    notify_shell();
    Ok(())
}

#[cfg(windows)]
fn notify_shell() {
    use windows_sys::Win32::UI::Shell::{SHChangeNotify, SHCNE_ASSOCCHANGED, SHCNF_IDLIST};
    // Tell Explorer to refresh file association icons.
    unsafe {
        SHChangeNotify(
            SHCNE_ASSOCCHANGED as i32,
            SHCNF_IDLIST,
            std::ptr::null(),
            std::ptr::null(),
        );
    }
}

/// Intentionally empty: a custom AppUserModelID without RelaunchIconResource
/// makes Windows show a generic window thumbnail on the taskbar.
#[cfg(windows)]
pub fn set_app_user_model_id() {}

/// Apply the icon embedded in this exe to the HWND (title + taskbar).
///
/// Sets both `WM_SETICON` and the window-class icons (`GCLP_HICON*`). The
/// taskbar often reads the class icon; RGBA-created icons alone are not enough.
#[cfg(windows)]
pub fn apply_hwnd_icons(hwnd: windows_sys::Win32::Foundation::HWND) {
    use std::os::windows::ffi::OsStrExt;

    use windows_sys::Win32::UI::Shell::ExtractIconExW;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        SendMessageW, SetClassLongPtrW, GCLP_HICON, GCLP_HICONSM, HICON, ICON_BIG, ICON_SMALL,
        WM_SETICON,
    };

    let Ok(exe) = std::env::current_exe() else {
        return;
    };
    let wide: Vec<u16> = exe
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();

    let mut large: HICON = std::ptr::null_mut();
    let mut small: HICON = std::ptr::null_mut();
    let n = unsafe { ExtractIconExW(wide.as_ptr(), 0, &mut large, &mut small, 1) };
    if n == 0 {
        return;
    }

    unsafe {
        if !large.is_null() {
            SendMessageW(hwnd, WM_SETICON, ICON_BIG as usize, large as isize);
            SetClassLongPtrW(hwnd, GCLP_HICON, large as isize);
        }
        if !small.is_null() {
            SendMessageW(hwnd, WM_SETICON, ICON_SMALL as usize, small as isize);
            SetClassLongPtrW(hwnd, GCLP_HICONSM, small as isize);
        } else if !large.is_null() {
            // Fallback: reuse large for small class icon.
            SendMessageW(hwnd, WM_SETICON, ICON_SMALL as usize, large as isize);
            SetClassLongPtrW(hwnd, GCLP_HICONSM, large as isize);
        }
    }
}

#[cfg(not(windows))]
pub fn set_app_user_model_id() {}

#[cfg(not(windows))]
pub fn apply_hwnd_icons(_hwnd: isize) {}

#[cfg(windows)]
pub fn report(title: &str, body: &str) {
    use std::io::Write;

    attach_parent_console();
    let wrote = std::fs::OpenOptions::new()
        .write(true)
        .open("CONOUT$")
        .and_then(|mut f| writeln!(f, "{title}: {body}"))
        .is_ok();
    if !wrote {
        show_message(title, body);
    }
}

#[cfg(windows)]
fn show_message(title: &str, body: &str) {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::UI::WindowsAndMessaging::{MessageBoxW, MB_OK};

    fn wide(s: &str) -> Vec<u16> {
        std::ffi::OsStr::new(s)
            .encode_wide()
            .chain(std::iter::once(0))
            .collect()
    }
    let title_w = wide(title);
    let body_w = wide(body);
    unsafe {
        MessageBoxW(
            std::ptr::null_mut(),
            body_w.as_ptr(),
            title_w.as_ptr(),
            MB_OK,
        );
    }
}

#[cfg(windows)]
fn attach_parent_console() {
    use windows_sys::Win32::System::Console::AttachConsole;
    // ATTACH_PARENT_PROCESS = -1
    unsafe {
        let _ = AttachConsole(u32::MAX);
    }
}

#[cfg(not(windows))]
pub fn register() -> Result<(), String> {
    Err("file association is Windows-only".into())
}

#[cfg(not(windows))]
pub fn unregister() -> Result<(), String> {
    Err("file association is Windows-only".into())
}

#[cfg(not(windows))]
pub fn report(title: &str, body: &str) {
    eprintln!("{title}: {body}");
}
