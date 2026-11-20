use std::{
    cell::Cell,
    ptr::null_mut,
    sync::{Arc, Mutex, OnceLock},
    thread,
    time::Duration,
};

use windows_sys::Win32::{
    Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM},
    Graphics::Gdi::*,
    System::LibraryLoader::GetModuleHandleW,
    UI::Controls::{DRAWITEMSTRUCT, ODS_FOCUS, ODS_SELECTED},
    UI::Input::KeyboardAndMouse::{ReleaseCapture, SetCapture},
    UI::WindowsAndMessaging::*,
};
use windows_sys::w;

use super::recorder::{CaptureRegion, Recorder, Recording};

const CLASS: *const u16 = w!("MegaImageEditorScreenCapture");
const BAR_HEIGHT: i32 = 34;
const EDGE: i32 = 8;
const BORDER: i32 = 2;
const TOOLBAR_LABEL_X: i32 = 164;
const RECORDING_LABEL_X: i32 = 238;
const CORNER: i32 = 28;
const MIN_WIDTH: i32 = 260;
const MIN_HEIGHT: i32 = 125;
const PRIMARY: usize = 1;
const SECONDARY: usize = 2;
const MINIMIZE: usize = 3;
const RESET_SIZE: usize = 4;
const ACTION: u32 = WM_APP + 1;
const SHOW_FRAME: u32 = WM_APP + 2;
const SELECT_TITLE: &str = "Screen recording — move and resize this frame";

struct Frame {
    hwnd: HWND,
    primary: HWND,
    secondary: HWND,
    reset: HWND,
    state: Box<FrameState>,
    published_window: Arc<Mutex<usize>>,
}

#[derive(Default)]
struct FrameState {
    region: CaptureRegion,
    resize: Cell<Option<ResizeDrag>>,
    shaping: Cell<bool>,
    recording_size: Cell<Option<RecordingSize>>,
}

#[derive(Clone, Copy)]
struct RecordingSize {
    capture: [i32; 2],
    window: [i32; 2],
}

#[derive(Clone, Copy)]
struct ResizeDrag {
    hit: u32,
    start: POINT,
    bounds: RECT,
}

impl ResizeDrag {
    fn bounds_at(self, point: POINT, capture_size: Option<[i32; 2]>) -> RECT {
        let dx = point.x - self.start.x;
        let dy = point.y - self.start.y;
        if dx == 0 && dy == 0 {
            return self.bounds;
        }
        let horizontal = match self.hit {
            HTLEFT | HTTOPLEFT | HTBOTTOMLEFT => -1,
            HTRIGHT | HTTOPRIGHT | HTBOTTOMRIGHT => 1,
            _ => 0,
        };
        let vertical = match self.hit {
            HTTOP | HTTOPLEFT | HTTOPRIGHT => -1,
            HTBOTTOM | HTBOTTOMLEFT | HTBOTTOMRIGHT => 1,
            _ => 0,
        };
        let RECT {
            left,
            top,
            right,
            bottom,
        } = self.bounds;
        let old_width = right - left;
        let old_height = bottom - top;
        let mut width = (old_width + dx * horizontal).max(MIN_WIDTH);
        let mut height = (old_height + dy * vertical).max(MIN_HEIGHT);
        if let Some([capture_w, capture_h]) = capture_size {
            let insets_x = 2 * EDGE;
            let insets_y = BAR_HEIGHT + 2 * EDGE;
            let ratio_x = f64::from(width - insets_x) / f64::from(capture_w);
            let ratio_y = f64::from(height - insets_y) / f64::from(capture_h);
            let x_delta = f64::from(width - old_width).abs() / f64::from(capture_w);
            let y_delta = f64::from(height - old_height).abs() / f64::from(capture_h);
            let scale = if vertical == 0 || (horizontal != 0 && x_delta >= y_delta) {
                ratio_x
            } else {
                ratio_y
            };
            let min_w = (MIN_WIDTH - insets_x + 1) & !1;
            let min_h = (MIN_HEIGHT - insets_y + 1) & !1;
            let scale = scale
                .max(f64::from(min_w) / f64::from(capture_w))
                .max(f64::from(min_h) / f64::from(capture_h));
            let even = |size| (f64::from(size) * scale / 2.0).round() as i32 * 2;
            width = even(capture_w) + insets_x;
            height = even(capture_h) + insets_y;
        }
        let anchor = |start, previous, next, edge| match edge {
            -1 => start + previous - next,
            0 => start + (previous - next) / 2,
            _ => start,
        };
        let left = anchor(left, old_width, width, horizontal);
        let top = anchor(top, old_height, height, vertical);
        RECT {
            left,
            top,
            right: left + width,
            bottom: top + height,
        }
    }
}

impl Frame {
    fn open(published_window: Arc<Mutex<usize>>) -> Result<Self, String> {
        unsafe {
            let instance = GetModuleHandleW(null_mut());
            static REGISTERED: OnceLock<Result<(), String>> = OnceLock::new();
            REGISTERED
                .get_or_init(|| {
                    let class = WNDCLASSW {
                        style: CS_HREDRAW | CS_VREDRAW,
                        lpfnWndProc: Some(window_proc),
                        hInstance: instance,
                        hCursor: LoadCursorW(null_mut(), IDC_ARROW),
                        lpszClassName: CLASS,
                        ..std::mem::zeroed()
                    };
                    if RegisterClassW(&class) == 0 {
                        return Err(windows_error("Cannot register capture frame"));
                    }
                    Ok(())
                })
                .clone()?;

            let mut pointer = POINT { x: 0, y: 0 };
            GetCursorPos(&mut pointer);
            let monitor = MonitorFromPoint(pointer, MONITOR_DEFAULTTONEAREST);
            let mut info: MONITORINFO = std::mem::zeroed();
            info.cbSize = size_of::<MONITORINFO>() as u32;
            if GetMonitorInfoW(monitor, &mut info) == 0 {
                return Err(windows_error("Cannot locate capture monitor"));
            }
            let RECT {
                left,
                top,
                right,
                bottom,
            } = info.rcWork;
            let width = (right - left).min(672);
            let height = (bottom - top).min(450);
            let title = wide(SELECT_TITLE);
            let hwnd = CreateWindowExW(
                WS_EX_TOPMOST | WS_EX_APPWINDOW,
                CLASS,
                title.as_ptr(),
                WS_POPUP | WS_SYSMENU | WS_MINIMIZEBOX | WS_CLIPCHILDREN,
                left + (right - left - width) / 2,
                top + (bottom - top - height) / 2,
                width,
                height,
                null_mut(),
                null_mut(),
                instance,
                null_mut(),
            );
            if hwnd.is_null() {
                return Err(windows_error("Cannot create capture frame"));
            }
            let mut frame = Self {
                hwnd,
                primary: null_mut(),
                secondary: null_mut(),
                reset: null_mut(),
                state: Box::default(),
                published_window,
            };
            update_window_region(hwnd)?;
            let button = |label, x, width, id| {
                CreateWindowExW(
                    0,
                    w!("BUTTON"),
                    label,
                    WS_CHILD | WS_TABSTOP | BS_OWNERDRAW as u32,
                    x,
                    5,
                    width,
                    24,
                    hwnd,
                    id as _,
                    instance,
                    null_mut(),
                )
            };
            frame.primary = button(w!("Record"), 8, 58, PRIMARY);
            frame.secondary = button(w!("Cancel"), 70, 58, SECONDARY);
            let minimize = button(w!("−"), 132, 22, MINIMIZE);
            frame.reset = button(w!("Reset Size"), 158, 70, RESET_SIZE);
            if [frame.primary, frame.secondary, minimize, frame.reset]
                .iter()
                .any(|button| button.is_null())
            {
                return Err(windows_error("Cannot create recording controls"));
            }
            for button in [frame.primary, frame.secondary, minimize] {
                ShowWindow(button, SW_SHOW);
            }
            // The boxed state has a stable address until DestroyWindow returns.
            SetWindowLongPtrW(
                hwnd,
                GWLP_USERDATA,
                (&*frame.state as *const FrameState) as isize,
            );
            ShowWindow(hwnd, SW_SHOW);
            frame.state.region.update(capture_area(hwnd)?);
            *frame.published_window.lock().unwrap() = hwnd as usize;
            Ok(frame)
        }
    }

    fn title(&self, title: &str) {
        unsafe {
            SetWindowTextW(self.hwnd, wide(title).as_ptr());
            let label = RECT {
                left: TOOLBAR_LABEL_X,
                top: 0,
                right: i32::MAX,
                bottom: BAR_HEIGHT,
            };
            InvalidateRect(self.hwnd, &label, 0);
        }
    }

    fn recording(&self) {
        unsafe {
            let [_, _, width, height] = self.state.region.area();
            let mut outer: RECT = std::mem::zeroed();
            if GetWindowRect(self.hwnd, &mut outer) != 0 {
                self.state.recording_size.set(Some(RecordingSize {
                    capture: [width, height],
                    window: [outer.right - outer.left, outer.bottom - outer.top],
                }));
            }
            SetWindowTextW(self.primary, w!("Pause"));
            SetWindowTextW(self.secondary, w!("Finish"));
            ShowWindow(self.reset, SW_SHOW);
            InvalidateRect(self.primary, null_mut(), 0);
            InvalidateRect(self.secondary, null_mut(), 0);
        }
    }

    fn pause(&self, paused: bool) {
        unsafe {
            SetWindowTextW(
                self.primary,
                if paused { w!("Resume") } else { w!("Pause") },
            );
            InvalidateRect(self.primary, null_mut(), 0);
        }
    }
}

impl Drop for Frame {
    fn drop(&mut self) {
        *self.published_window.lock().unwrap() = 0;
        unsafe { DestroyWindow(self.hwnd) };
    }
}

pub fn show(window: &Mutex<usize>) {
    let hwnd = *window.lock().unwrap() as HWND;
    if !hwnd.is_null() {
        unsafe {
            // Keep the activation request tied to the user's click in the editor.
            SetForegroundWindow(hwnd);
            PostMessageW(hwnd, SHOW_FRAME, 0, 0);
        }
    }
}

pub fn run(
    published_window: Arc<Mutex<usize>>,
    saving: impl FnOnce(),
) -> Result<Option<Recording>, String> {
    let frame = Frame::open(published_window)?;
    let mut recorder: Option<Recorder> = None;
    let mut paused = false;
    let mut last_seconds = None;
    'window: loop {
        unsafe {
            let mut message: MSG = std::mem::zeroed();
            while PeekMessageW(&mut message, null_mut(), 0, 0, PM_REMOVE) != 0 {
                match message.message {
                    ACTION if message.wParam == PRIMARY => {
                        if let Some(recorder) = &recorder {
                            paused = recorder.toggle_pause();
                            last_seconds = None;
                            frame.pause(paused);
                        } else {
                            match Recorder::start(frame.state.region.clone()) {
                                Ok(job) => {
                                    recorder = Some(job);
                                    frame.recording();
                                }
                                Err(error) => frame.title(&error),
                            }
                        }
                    }
                    ACTION | WM_QUIT => break 'window,
                    _ => {
                        TranslateMessage(&message);
                        DispatchMessageW(&message);
                    }
                }
            }
        }
        if let Some(recorder) = &recorder {
            if recorder.finishing() {
                break;
            }
            let seconds = recorder.duration().as_secs();
            if last_seconds != Some(seconds) {
                let status = if paused { "Paused" } else { "Recording" };
                frame.title(&format!(
                    "{status}  {:02}:{:02}",
                    seconds / 60,
                    seconds % 60
                ));
                last_seconds = Some(seconds);
            }
        }
        thread::sleep(Duration::from_millis(15));
    }
    if let Some(recorder) = recorder {
        recorder.stop();
        drop(frame);
        saving();
        recorder.finish().map(Some)
    } else {
        Ok(None)
    }
}

unsafe extern "system" fn window_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    unsafe {
        let state = (GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const FrameState).as_ref();
        let recording_size = state.and_then(|state| state.recording_size.get());
        let label_x = if recording_size.is_some() {
            RECORDING_LABEL_X
        } else {
            TOOLBAR_LABEL_X
        };
        match message {
            WM_COMMAND if wparam == MINIMIZE => {
                ShowWindow(hwnd, SW_MINIMIZE);
                0
            }
            WM_COMMAND if wparam == RESET_SIZE => {
                if let Some(RecordingSize {
                    window: [width, height],
                    ..
                }) = recording_size
                {
                    SetWindowPos(
                        hwnd,
                        null_mut(),
                        0,
                        0,
                        width,
                        height,
                        SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE,
                    );
                }
                0
            }
            WM_COMMAND if [PRIMARY, SECONDARY].contains(&wparam) => {
                PostMessageW(hwnd, ACTION, wparam, 0);
                0
            }
            WM_CLOSE => {
                PostMessageW(hwnd, ACTION, SECONDARY, 0);
                0
            }
            SHOW_FRAME => {
                if IsIconic(hwnd) != 0 {
                    ShowWindow(hwnd, SW_RESTORE);
                }
                SetWindowPos(
                    hwnd,
                    HWND_TOPMOST,
                    0,
                    0,
                    0,
                    0,
                    SWP_NOMOVE | SWP_NOSIZE | SWP_SHOWWINDOW,
                );
                SetForegroundWindow(hwnd);
                0
            }
            WM_MOUSEACTIVATE => MA_ACTIVATE as LRESULT,
            WM_NCHITTEST => {
                let hit = DefWindowProcW(hwnd, message, wparam, lparam);
                let mut point = mouse_point(lparam);
                if hit == HTCLIENT as LRESULT
                    && ScreenToClient(hwnd, &mut point) != 0
                    && (0..BAR_HEIGHT).contains(&point.y)
                    && (point.x >= label_x - 8 || !(5..29).contains(&point.y))
                {
                    HTCAPTION as LRESULT
                } else {
                    hit
                }
            }
            WM_SIZE if wparam != SIZE_MINIMIZED as usize => {
                if let Some(state) = state {
                    if !state.shaping.replace(true) {
                        if let Err(error) = update_window_region(hwnd) {
                            log::warn!("{error}");
                        }
                        state.shaping.set(false);
                    }
                }
                DefWindowProcW(hwnd, message, wparam, lparam)
            }
            WM_WINDOWPOSCHANGED => {
                let result = DefWindowProcW(hwnd, message, wparam, lparam);
                if let Some(state) = state.filter(|_| IsIconic(hwnd) == 0) {
                    if let Ok(area) = capture_area(hwnd) {
                        state.region.update(area);
                    }
                }
                result
            }
            WM_SETCURSOR if lparam as u16 as u32 == HTCLIENT => {
                let mut pointer = POINT { x: 0, y: 0 };
                if GetCursorPos(&mut pointer) != 0 && ScreenToClient(hwnd, &mut pointer) != 0 {
                    let hit = state
                        .and_then(|state| state.resize.get().map(|drag| drag.hit))
                        .or_else(|| client_resize_hit(hwnd, pointer));
                    if let Some(hit) = hit {
                        let cursor = match hit {
                            HTTOPLEFT | HTBOTTOMRIGHT => IDC_SIZENWSE,
                            HTTOPRIGHT | HTBOTTOMLEFT => IDC_SIZENESW,
                            HTTOP | HTBOTTOM => IDC_SIZENS,
                            _ => IDC_SIZEWE,
                        };
                        SetCursor(LoadCursorW(null_mut(), cursor));
                        return 1;
                    }
                }
                DefWindowProcW(hwnd, message, wparam, lparam)
            }
            WM_LBUTTONDOWN => {
                let mut start = mouse_point(lparam);
                if let Some((state, hit)) = state.zip(client_resize_hit(hwnd, start)) {
                    let mut bounds: RECT = std::mem::zeroed();
                    if ClientToScreen(hwnd, &mut start) != 0
                        && GetWindowRect(hwnd, &mut bounds) != 0
                    {
                        state.resize.set(Some(ResizeDrag { hit, start, bounds }));
                        SetCapture(hwnd);
                    }
                }
                0
            }
            WM_MOUSEMOVE => {
                if let Some(drag) = state.and_then(|state| state.resize.get()) {
                    let mut point = mouse_point(lparam);
                    if ClientToScreen(hwnd, &mut point) != 0 {
                        let bounds = drag.bounds_at(point, recording_size.map(|size| size.capture));
                        SetWindowPos(
                            hwnd,
                            null_mut(),
                            bounds.left,
                            bounds.top,
                            bounds.right - bounds.left,
                            bounds.bottom - bounds.top,
                            SWP_NOZORDER | SWP_NOACTIVATE,
                        );
                    }
                }
                0
            }
            WM_LBUTTONUP | WM_CANCELMODE | WM_CAPTURECHANGED => {
                if let Some(state) = state {
                    if state.resize.take().is_some() && message != WM_CAPTURECHANGED {
                        ReleaseCapture();
                    }
                }
                DefWindowProcW(hwnd, message, wparam, lparam)
            }
            WM_GETMINMAXINFO => {
                let limits = &mut *(lparam as *mut MINMAXINFO);
                limits.ptMinTrackSize = POINT {
                    x: MIN_WIDTH,
                    y: MIN_HEIGHT,
                };
                0
            }
            WM_PAINT => {
                let mut paint: PAINTSTRUCT = std::mem::zeroed();
                let dc = BeginPaint(hwnd, &mut paint);
                let mut client: RECT = std::mem::zeroed();
                GetClientRect(hwnd, &mut client);
                let bar = RECT {
                    bottom: BAR_HEIGHT,
                    ..client
                };
                let background = CreateSolidBrush(0x0021_2121);
                FillRect(dc, &bar, background);
                DeleteObject(background);
                draw_label(
                    hwnd,
                    dc,
                    RECT {
                        left: label_x,
                        right: client.right - 10,
                        ..bar
                    },
                    DT_LEFT,
                );
                let accent = CreateSolidBrush(0x0060_85ff);
                for strip in outline_parts(client) {
                    FillRect(dc, &strip, accent);
                }
                DeleteObject(accent);
                EndPaint(hwnd, &paint);
                0
            }
            WM_DRAWITEM => {
                let item = &*(lparam as *const DRAWITEMSTRUCT);
                let color = if item.itemState & ODS_SELECTED != 0 {
                    0x0055_5555
                } else if item.CtlID as usize == PRIMARY {
                    0x0048_4848
                } else {
                    0x0034_3434
                };
                let brush = CreateSolidBrush(color);
                FillRect(item.hDC, &item.rcItem, brush);
                DeleteObject(brush);
                draw_label(item.hwndItem, item.hDC, item.rcItem, DT_CENTER);
                if item.itemState & ODS_FOCUS != 0 {
                    DrawFocusRect(item.hDC, &item.rcItem);
                }
                1
            }
            _ => DefWindowProcW(hwnd, message, wparam, lparam),
        }
    }
}

fn draw_label(hwnd: HWND, dc: HDC, mut bounds: RECT, align: u32) {
    unsafe {
        let mut label = [0u16; 256];
        let length = GetWindowTextW(hwnd, label.as_mut_ptr(), label.len() as i32);
        let font = SelectObject(dc, GetStockObject(DEFAULT_GUI_FONT));
        SetTextColor(dc, 0x00eb_ebeb);
        SetBkMode(dc, TRANSPARENT as i32);
        DrawTextW(
            dc,
            label.as_ptr(),
            length,
            &mut bounds,
            align | DT_VCENTER | DT_SINGLELINE | DT_END_ELLIPSIS | DT_NOPREFIX,
        );
        SelectObject(dc, font);
    }
}

fn outline_parts(client: RECT) -> [RECT; 8] {
    let [x, y] = [client.right, client.bottom];
    let [left, top, right, bottom] = [EDGE, BAR_HEIGHT + EDGE, x - EDGE, y - EDGE];
    [
        RECT {
            left,
            top: top - BORDER,
            right,
            bottom: top,
        },
        RECT {
            left,
            top: bottom,
            right,
            bottom: bottom + BORDER,
        },
        RECT {
            left: left - BORDER,
            top,
            right: left,
            bottom,
        },
        RECT {
            left: right,
            top,
            right: right + BORDER,
            bottom,
        },
        RECT {
            left: 0,
            top: BAR_HEIGHT,
            right: left,
            bottom: top,
        },
        RECT {
            left: right,
            top: BAR_HEIGHT,
            right: x,
            bottom: top,
        },
        RECT {
            left: 0,
            top: bottom,
            right: left,
            bottom: y,
        },
        RECT {
            left: right,
            top: bottom,
            right: x,
            bottom: y,
        },
    ]
}

fn update_window_region(hwnd: HWND) -> Result<(), String> {
    unsafe {
        let mut client: RECT = std::mem::zeroed();
        if GetClientRect(hwnd, &mut client) == 0 {
            return Err(windows_error("Cannot measure capture frame"));
        }
        // WS_POPUP has no nonclient frame. Region and paint share client coordinates.
        let frame = CreateRectRgn(0, 0, client.right, BAR_HEIGHT);
        let part = CreateRectRgn(0, 0, 0, 0);
        let combined = !frame.is_null()
            && !part.is_null()
            && outline_parts(client).into_iter().all(|rect| {
                SetRectRgn(part, rect.left, rect.top, rect.right, rect.bottom) != 0
                    && CombineRgn(frame, frame, part, RGN_OR) != 0
            });
        if !part.is_null() {
            DeleteObject(part);
        }
        if !combined || SetWindowRgn(hwnd, frame, 1) == 0 {
            if !frame.is_null() {
                DeleteObject(frame);
            }
            return Err(windows_error("Cannot shape capture frame"));
        }
        Ok(())
    }
}

fn capture_area(hwnd: HWND) -> Result<[i32; 4], String> {
    unsafe {
        let mut rect = std::mem::zeroed::<RECT>();
        let mut origin = POINT {
            x: EDGE,
            y: BAR_HEIGHT + EDGE,
        };
        if GetClientRect(hwnd, &mut rect) == 0 || ClientToScreen(hwnd, &mut origin) == 0 {
            return Err(windows_error("Cannot locate capture area"));
        }
        let width = (rect.right - 2 * EDGE) & !1;
        let height = (rect.bottom - BAR_HEIGHT - 2 * EDGE) & !1;
        Ok([origin.x, origin.y, width, height])
    }
}

fn mouse_point(lparam: LPARAM) -> POINT {
    POINT {
        x: lparam as i16 as i32,
        y: (lparam >> 16) as i16 as i32,
    }
}

fn client_resize_hit(hwnd: HWND, point: POINT) -> Option<u32> {
    unsafe {
        let mut client: RECT = std::mem::zeroed();
        if GetClientRect(hwnd, &mut client) == 0 {
            return None;
        }
        client.top = BAR_HEIGHT;
        resize_hit(point, client, EDGE)
    }
}

fn resize_hit(point: POINT, bounds: RECT, edge: i32) -> Option<u32> {
    let POINT { x, y } = point;
    if x < bounds.left || x >= bounds.right || y < bounds.top || y >= bounds.bottom {
        return None;
    }
    let left = x < bounds.left + edge;
    let right = x >= bounds.right - edge;
    let top = y < bounds.top + edge;
    let bottom = y >= bounds.bottom - edge;
    let near_left = x < bounds.left + CORNER;
    let near_right = x >= bounds.right - CORNER;
    let near_top = y < bounds.top + CORNER;
    let near_bottom = y >= bounds.bottom - CORNER;
    let hit = if (top && near_left) || (left && near_top) {
        HTTOPLEFT
    } else if (top && near_right) || (right && near_top) {
        HTTOPRIGHT
    } else if (bottom && near_left) || (left && near_bottom) {
        HTBOTTOMLEFT
    } else if (bottom && near_right) || (right && near_bottom) {
        HTBOTTOMRIGHT
    } else {
        match (left, right, top, bottom) {
            (true, _, _, _) => HTLEFT,
            (_, true, _, _) => HTRIGHT,
            (_, _, true, _) => HTTOP,
            (_, _, _, true) => HTBOTTOM,
            _ => return None,
        }
    };
    Some(hit)
}

fn wide(label: &str) -> Vec<u16> {
    label.encode_utf16().chain(std::iter::once(0)).collect()
}

fn windows_error(context: &str) -> String {
    format!("{context}: {}", std::io::Error::last_os_error())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn visible_frame_has_resize_handles_at_all_corners_and_edges() {
        let bounds = RECT {
            left: -300,
            top: 42,
            right: 100,
            bottom: 242,
        };
        for (x, y, hit) in [
            (-298, 44, HTTOPLEFT),
            (98, 44, HTTOPRIGHT),
            (-298, 239, HTBOTTOMLEFT),
            (98, 239, HTBOTTOMRIGHT),
            (-298, 140, HTLEFT),
            (98, 140, HTRIGHT),
            (-100, 44, HTTOP),
            (-100, 239, HTBOTTOM),
        ] {
            assert_eq!(resize_hit(POINT { x, y }, bounds, EDGE), Some(hit));
        }
        assert_eq!(resize_hit(POINT { x: -100, y: 140 }, bounds, EDGE), None);
    }

    #[test]
    fn recording_resize_locks_capture_aspect_without_moving_opposite_corner() {
        let drag = ResizeDrag {
            hit: HTBOTTOMRIGHT,
            start: POINT { x: 650, y: 400 },
            bounds: RECT {
                left: 10,
                top: 20,
                right: 666,
                bottom: 430,
            },
        };
        let pointer = POINT { x: 810, y: 420 };
        let free = drag.bounds_at(pointer, None);
        assert_eq!([free.right, free.bottom], [826, 450]);
        let locked = drag.bounds_at(pointer, Some([640, 360]));
        assert_eq!([locked.left, locked.top], [10, 20]);
        assert_eq!([locked.right, locked.bottom], [826, 520]);

        let side = ResizeDrag {
            hit: HTLEFT,
            ..drag
        };
        let locked = side.bounds_at(POINT { x: 810, y: 400 }, Some([640, 360]));
        assert_eq!(locked.right, drag.bounds.right);
        assert_eq!(
            locked.top + locked.bottom,
            drag.bounds.top + drag.bounds.bottom
        );
        assert_eq!(locked.bottom - locked.top - BAR_HEIGHT - 2 * EDGE, 270);
    }

    #[test]
    #[ignore = "requires an interactive Windows desktop and audio output"]
    fn live_recording_follows_frame_and_restores_it_after_minimizing() {
        let published = Arc::new(Mutex::new(0));
        let frame = Frame::open(Arc::clone(&published)).unwrap();
        let initial = frame.state.region.area();
        let recorder = Recorder::start(frame.state.region.clone()).unwrap();
        assert_eq!(unsafe { IsWindowVisible(frame.reset) }, 0);
        frame.recording();
        assert_ne!(unsafe { IsWindowVisible(frame.reset) }, 0);
        thread::sleep(Duration::from_millis(350));

        unsafe {
            let mut bounds: RECT = std::mem::zeroed();
            assert_ne!(GetWindowRect(frame.hwnd, &mut bounds), 0);
            let mut client: RECT = std::mem::zeroed();
            assert_ne!(GetClientRect(frame.hwnd, &mut client), 0);
            let grip = POINT {
                x: client.right - 4,
                y: client.bottom - 4,
            };
            let mut on_screen = grip;
            ClientToScreen(frame.hwnd, &mut on_screen);
            assert_eq!(
                WindowFromPoint(on_screen),
                frame.hwnd,
                "bottom-right grip accepts a click"
            );
            let left_edge = |offset| {
                WindowFromPoint(POINT {
                    x: initial[0] + offset,
                    y: initial[1] + initial[3] / 2,
                })
            };
            assert_eq!(left_edge(-1), frame.hwnd);
            assert_eq!(left_edge(-2), frame.hwnd);
            assert_ne!(left_edge(-3), frame.hwnd);
            assert_ne!(left_edge(0), frame.hwnd);
            assert_ne!(
                WindowFromPoint(POINT {
                    x: initial[0] + 50,
                    y: initial[1] + 50
                }),
                frame.hwnd
            );
            let pack = |x, y| (x as u16 as isize) | ((y as u16 as isize) << 16);
            SendMessageW(frame.hwnd, WM_LBUTTONDOWN, 0, pack(grip.x, grip.y));
            assert!(frame.state.resize.get().is_some());
            SendMessageW(frame.hwnd, WM_MOUSEMOVE, 1, pack(grip.x - 48, grip.y - 72));
            SendMessageW(frame.hwnd, WM_LBUTTONUP, 0, pack(grip.x - 48, grip.y - 72));
            assert!(frame.state.resize.get().is_none());
            let resized = frame.state.region.area();
            assert_eq!(resized[0..2], initial[0..2]);
            assert!(resized[2] < initial[2] && resized[3] < initial[3]);
            assert!(
                (resized[2] * initial[3] - resized[3] * initial[2]).abs()
                    <= initial[2] + initial[3]
            );
            assert_ne!(
                SetWindowPos(
                    frame.hwnd,
                    null_mut(),
                    bounds.left + 35,
                    bounds.top + 20,
                    0,
                    0,
                    SWP_NOZORDER | SWP_NOACTIVATE | SWP_NOSIZE
                ),
                0
            );
            let moved = frame.state.region.area();
            assert_eq!(moved[0..2], [initial[0] + 35, initial[1] + 20]);
            assert_eq!(moved[2..4], resized[2..4]);
            SendMessageW(frame.reset, BM_CLICK, 0, 0);
            let restored = frame.state.region.area();
            assert_eq!(restored[0..2], moved[0..2]);
            assert_eq!(restored[2..4], initial[2..4]);
            ShowWindow(frame.hwnd, SW_MINIMIZE);
            assert_ne!(IsIconic(frame.hwnd), 0);
            assert_eq!(frame.state.region.area(), restored);
            show(&published);
            let mut message: MSG = std::mem::zeroed();
            assert_ne!(
                PeekMessageW(&mut message, frame.hwnd, SHOW_FRAME, SHOW_FRAME, PM_REMOVE),
                0
            );
            DispatchMessageW(&message);
            assert_eq!(IsIconic(frame.hwnd), 0);
            assert_eq!(GetForegroundWindow(), frame.hwnd);
            assert_eq!(frame.state.region.area(), restored);

            let other = Frame::open(Arc::new(Mutex::new(0))).unwrap();
            SetForegroundWindow(other.hwnd);
            assert_eq!(GetForegroundWindow(), other.hwnd);
            show(&published);
            assert_ne!(
                PeekMessageW(&mut message, frame.hwnd, SHOW_FRAME, SHOW_FRAME, PM_REMOVE),
                0
            );
            DispatchMessageW(&message);
            assert_eq!(GetForegroundWindow(), frame.hwnd);
        }
        thread::sleep(Duration::from_millis(450));

        let recording = recorder.finish().unwrap();
        assert_eq!(recording.warning, None);
        let info = crate::video::probe_mp4(&recording.bytes).unwrap();
        assert_eq!(
            [info.width, info.height],
            [initial[2] as u32, initial[3] as u32]
        );
        assert!(info.has_audio);
    }
}
