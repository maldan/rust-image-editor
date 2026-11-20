use std::{ffi::c_void, ptr, slice};

use windows_sys::Win32::Graphics::Gdi::*;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    GetSystemMetrics, SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN, SM_XVIRTUALSCREEN, SM_YVIRTUALSCREEN,
};

pub struct Desktop {
    screen: HDC,
    memory: HDC,
    bitmap: HBITMAP,
    original: HGDIOBJ,
    pixels: *mut c_void,
    size: [i32; 2],
}

impl Desktop {
    pub fn new(size: [i32; 2]) -> Result<Self, String> {
        let [width, height] = size;
        let mut desktop = Self {
            screen: ptr::null_mut(),
            memory: ptr::null_mut(),
            bitmap: ptr::null_mut(),
            original: ptr::null_mut(),
            pixels: ptr::null_mut(),
            size,
        };
        unsafe {
            desktop.screen = GetDC(ptr::null_mut());
            if desktop.screen.is_null() {
                return Err("Cannot open desktop".into());
            }
            desktop.memory = CreateCompatibleDC(desktop.screen);
            if desktop.memory.is_null() {
                return Err("Cannot create capture DC".into());
            }
            let mut info: BITMAPINFO = std::mem::zeroed();
            info.bmiHeader.biSize = size_of::<BITMAPINFOHEADER>() as u32;
            info.bmiHeader.biWidth = width;
            info.bmiHeader.biHeight = -height;
            info.bmiHeader.biPlanes = 1;
            info.bmiHeader.biBitCount = 32;
            info.bmiHeader.biCompression = BI_RGB;
            desktop.bitmap = CreateDIBSection(
                desktop.screen,
                &info,
                DIB_RGB_COLORS,
                &mut desktop.pixels,
                ptr::null_mut(),
                0,
            );
            if desktop.bitmap.is_null() || desktop.pixels.is_null() {
                return Err("Cannot allocate capture bitmap".into());
            }
            desktop.original = SelectObject(desktop.memory, desktop.bitmap);
            if desktop.original.is_null() {
                return Err("Cannot select capture bitmap".into());
            }
        }
        Ok(desktop)
    }

    pub fn frame(&mut self, source: [i32; 4]) -> Result<Vec<u8>, String> {
        let [width, height] = self.size;
        unsafe {
            let desktop = [
                GetSystemMetrics(SM_XVIRTUALSCREEN),
                GetSystemMetrics(SM_YVIRTUALSCREEN),
                GetSystemMetrics(SM_CXVIRTUALSCREEN),
                GetSystemMetrics(SM_CYVIRTUALSCREEN),
            ];
            let mut ok = true;
            let crop = visible_crop(source, desktop, self.size);
            if !matches!(crop, Some((_, [0, 0, w, h])) if [w, h] == self.size) {
                ok = PatBlt(self.memory, 0, 0, width, height, BLACKNESS) != 0;
            }
            if let Some(([x, y, source_w, source_h], [left, top, target_w, target_h])) = crop {
                let raster = SRCCOPY | CAPTUREBLT;
                let copied = if [source_w, source_h] == [target_w, target_h] {
                    BitBlt(
                        self.memory,
                        left,
                        top,
                        target_w,
                        target_h,
                        self.screen,
                        x,
                        y,
                        raster,
                    )
                } else {
                    SetStretchBltMode(self.memory, COLORONCOLOR);
                    StretchBlt(
                        self.memory,
                        left,
                        top,
                        target_w,
                        target_h,
                        self.screen,
                        x,
                        y,
                        source_w,
                        source_h,
                        raster,
                    )
                };
                ok &= copied != 0;
            }
            if !ok || GdiFlush() == 0 {
                return Err(format!(
                    "Desktop capture failed: {}",
                    std::io::Error::last_os_error()
                ));
            }
            Ok(
                slice::from_raw_parts(self.pixels.cast(), width as usize * height as usize * 4)
                    .to_vec(),
            )
        }
    }
}

/// Keep the initial encoded size and map only visible desktop pixels into it.
fn visible_crop(
    [x, y, w, h]: [i32; 4],
    [screen_x, screen_y, screen_w, screen_h]: [i32; 4],
    [output_w, output_h]: [i32; 2],
) -> Option<([i32; 4], [i32; 4])> {
    let left = x.max(screen_x);
    let top = y.max(screen_y);
    let right = (x + w).min(screen_x + screen_w);
    let bottom = (y + h).min(screen_y + screen_h);
    if left >= right || top >= bottom {
        return None;
    }
    let scale = (f64::from(output_w) / f64::from(w)).min(f64::from(output_h) / f64::from(h));
    let map = |coordinate: i32, origin: i32, extent: i32, output: i32| {
        ((f64::from(output) - f64::from(extent) * scale) / 2.0
            + f64::from(coordinate - origin) * scale)
            .round() as i32
    };
    let dx = map(left, x, w, output_w);
    let dy = map(top, y, h, output_h);
    let dw = map(right, x, w, output_w) - dx;
    let dh = map(bottom, y, h, output_h) - dy;
    (dw > 0 && dh > 0).then_some(([left, top, right - left, bottom - top], [dx, dy, dw, dh]))
}

impl Drop for Desktop {
    fn drop(&mut self) {
        unsafe {
            if !self.original.is_null() {
                SelectObject(self.memory, self.original);
            }
            if !self.bitmap.is_null() {
                DeleteObject(self.bitmap);
            }
            if !self.memory.is_null() {
                DeleteDC(self.memory);
            }
            if !self.screen.is_null() {
                ReleaseDC(ptr::null_mut(), self.screen);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resized_capture_fits_unchanged_output_without_distortion() {
        assert_eq!(
            visible_crop([40, 80, 200, 200], [0, 0, 1920, 1080], [640, 360]),
            Some(([40, 80, 200, 200], [140, 0, 360, 360])),
        );
    }

    #[test]
    fn moved_capture_uses_latest_origin_and_pads_offscreen_pixels() {
        let desktop = [-1920, 0, 3840, 1080];
        assert_eq!(
            visible_crop([-960, 160, 640, 360], desktop, [640, 360]),
            Some(([-960, 160, 640, 360], [0, 0, 640, 360])),
        );
        assert_eq!(
            visible_crop([-2000, 160, 640, 360], desktop, [640, 360]),
            Some(([-1920, 160, 560, 360], [80, 0, 560, 360])),
        );
    }
}
