use std::sync::Arc;

use tauri::State;

use crate::state::AppState;

use super::shared::CmdResult;
#[cfg(not(target_os = "android"))]
use super::shared::err;

#[cfg(windows)]
mod win32_fullscreen {
    use anyhow::{anyhow, Result};
    use once_cell::sync::Lazy;
    use parking_lot::Mutex;
    use windows::Win32::Foundation::{HWND, LPARAM, RECT, WPARAM};
    use windows::Win32::Graphics::Gdi::{
        GetMonitorInfoW, MonitorFromWindow, MONITORINFO, MONITOR_DEFAULTTONEAREST,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        GetClientRect, GetWindowLongPtrW, GetWindowPlacement, SendMessageW, SetWindowLongPtrW,
        SetWindowPlacement, SetWindowPos, ShowWindow, GWL_STYLE, HWND_TOP, SIZE_MAXIMIZED,
        SIZE_RESTORED, SWP_FRAMECHANGED, SWP_NOMOVE, SWP_NOOWNERZORDER, SWP_NOSIZE, SW_RESTORE,
        SW_SHOWMAXIMIZED, WINDOWPLACEMENT, WM_SIZE, WS_MAXIMIZE, WS_OVERLAPPEDWINDOW, WS_POPUP,
    };

    #[derive(Clone, Copy)]
    struct Saved {
        style: isize,
        placement: WINDOWPLACEMENT,
    }

    static SAVED: Lazy<Mutex<Option<Saved>>> = Lazy::new(|| Mutex::new(None));

    fn restore(hwnd: HWND, saved: Saved) -> Result<()> {
        let was_maximized = saved.placement.showCmd == SW_SHOWMAXIMIZED.0 as u32;
        unsafe {
            SetWindowLongPtrW(hwnd, GWL_STYLE, saved.style);
            SetWindowPlacement(hwnd, &saved.placement)
                .map_err(|e| anyhow!("SetWindowPlacement: {e}"))?;
            SetWindowPos(
                hwnd,
                Some(HWND_TOP),
                0, 0, 0, 0,
                SWP_FRAMECHANGED | SWP_NOOWNERZORDER | SWP_NOMOVE | SWP_NOSIZE,
            )
            .map_err(|e| anyhow!("SetWindowPos exit: {e}"))?;

            let mut client: RECT = std::mem::zeroed();
            let _ = GetClientRect(hwnd, &mut client);
            let w = (client.right - client.left).max(0) as u32;
            let h = (client.bottom - client.top).max(0) as u32;
            let lparam = LPARAM(((h << 16) | (w & 0xFFFF)) as isize);
            let wparam = WPARAM(if was_maximized {
                SIZE_MAXIMIZED as usize
            } else {
                SIZE_RESTORED as usize
            });
            let _ = SendMessageW(hwnd, WM_SIZE, Some(wparam), Some(lparam));
        }
        Ok(())
    }

    pub fn enter(hwnd_raw: isize) -> Result<()> {
        let hwnd = HWND(hwnd_raw as *mut _);
        unsafe {
            let style = GetWindowLongPtrW(hwnd, GWL_STYLE);
            if (style & WS_POPUP.0 as isize) != 0 {
                return Ok(());
            }

            if SAVED.lock().is_none() {
                let mut placement: WINDOWPLACEMENT = std::mem::zeroed();
                placement.length = std::mem::size_of::<WINDOWPLACEMENT>() as u32;
                GetWindowPlacement(hwnd, &mut placement)
                    .map_err(|e| anyhow!("GetWindowPlacement: {e}"))?;
                *SAVED.lock() = Some(Saved { style, placement });
            }

            let monitor = MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST);
            let mut mi: MONITORINFO = std::mem::zeroed();
            mi.cbSize = std::mem::size_of::<MONITORINFO>() as u32;
            if !GetMonitorInfoW(monitor, &mut mi).as_bool() {
                return Err(anyhow!("GetMonitorInfoW failed"));
            }
            let r = mi.rcMonitor;

            let saved_was_max = SAVED
                .lock()
                .as_ref()
                .map(|s| s.placement.showCmd == SW_SHOWMAXIMIZED.0 as u32)
                .unwrap_or(false);
            if saved_was_max {
                let _ = ShowWindow(hwnd, SW_RESTORE);
            }

            let strip = (WS_OVERLAPPEDWINDOW.0 | WS_MAXIMIZE.0) as isize;
            let new_style = (style & !strip) | WS_POPUP.0 as isize;
            SetWindowLongPtrW(hwnd, GWL_STYLE, new_style);

            SetWindowPos(
                hwnd,
                Some(HWND_TOP),
                r.left,
                r.top,
                r.right - r.left,
                r.bottom - r.top,
                SWP_FRAMECHANGED | SWP_NOOWNERZORDER,
            )
            .map_err(|e| anyhow!("SetWindowPos enter: {e}"))?;
        }
        Ok(())
    }

    pub fn exit(hwnd_raw: isize) -> Result<()> {
        let saved = match SAVED.lock().take() {
            Some(s) => s,
            None => return Ok(()),
        };
        let hwnd = HWND(hwnd_raw as *mut _);
        restore(hwnd, saved)
    }
}

#[tauri::command]
pub async fn window_set_fullscreen(
    app: tauri::AppHandle,
    state: State<'_, Arc<AppState>>,
    fullscreen: bool,
) -> CmdResult<()> {
    #[cfg(windows)]
    {
        let _ = &app;
        let hwnd = state
            .main_hwnd()
            .ok_or_else(|| "Finestra principale non disponibile".to_string())?;
        if fullscreen {
            win32_fullscreen::enter(hwnd).map_err(err)
        } else {
            win32_fullscreen::exit(hwnd).map_err(err)
        }
    }
    #[cfg(target_os = "android")]
    {
        // The activity already fills the screen; the external player handles
        // its own fullscreen.
        let _ = (&app, &state, fullscreen);
        Ok(())
    }
    #[cfg(not(any(windows, target_os = "android")))]
    {
        // GTK (and macOS) handle borderless fullscreen natively.
        use tauri::Manager;
        let _ = &state;
        let window = app
            .get_webview_window("main")
            .ok_or_else(|| "Finestra principale non disponibile".to_string())?;
        window.set_fullscreen(fullscreen).map_err(err)
    }
}

/// Android: turns the phone to landscape (immersive) while the player overlay
/// is open, from the moment it opens rather than when the video surface shows.
/// Nothing to do on desktop.
#[tauri::command]
pub async fn player_mode(app: tauri::AppHandle, on: bool) -> CmdResult<()> {
    #[cfg(target_os = "android")]
    {
        use tauri::Manager;
        app.state::<siiishub_android_player::AndroidMpv<tauri::Wry>>()
            .set_player_mode(on)
    }
    #[cfg(not(target_os = "android"))]
    {
        let _ = (app, on);
        Ok(())
    }
}
