//! Screenshot of the active window, taken when a bug report starts so it shows
//! what the speaker is describing. Windows only: other platforms capture
//! nothing and bug reports are saved without a picture.

use log::{debug, warn};
use std::sync::Mutex;

/// A captured window, as 8-bit RGBA.
pub struct Shot {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

/// The capture belonging to the bug report in progress. Every bug report
/// start overwrites it, so a cancelled report can't leak into the next one.
static PENDING: Mutex<Option<Shot>> = Mutex::new(None);

fn pending() -> std::sync::MutexGuard<'static, Option<Shot>> {
    PENDING.lock().unwrap_or_else(|e| e.into_inner())
}

/// Captures the foreground window now and keeps it for [`take`]. Call before
/// Handy puts its own overlay on screen.
pub fn capture_now() {
    let shot = capture_foreground();
    if shot.is_none() {
        debug!("No screenshot captured for the bug report");
    }
    *pending() = shot;
}

/// The capture from the last [`capture_now`], if there is one.
pub fn take() -> Option<Shot> {
    pending().take()
}

/// PNG data for a capture; `None` if it can't be encoded.
pub fn encode_png(shot: &Shot) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    let mut encoder = png::Encoder::new(&mut out, shot.width, shot.height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    // Fast: a screenshot is saved while the user waits, size matters less.
    encoder.set_compression(png::Compression::Fast);
    let result = encoder
        .write_header()
        .and_then(|mut writer| writer.write_image_data(&shot.rgba));
    match result {
        Ok(()) => Some(out),
        Err(e) => {
            warn!("Couldn't encode the bug report screenshot: {e}");
            None
        }
    }
}

#[cfg(not(windows))]
fn capture_foreground() -> Option<Shot> {
    None
}

#[cfg(windows)]
fn capture_foreground() -> Option<Shot> {
    use std::ffi::c_void;
    use std::mem::size_of;
    use windows::Win32::Foundation::RECT;
    use windows::Win32::Graphics::Dwm::{DwmGetWindowAttribute, DWMWA_EXTENDED_FRAME_BOUNDS};
    use windows::Win32::Graphics::Gdi::{
        BitBlt, CreateCompatibleBitmap, CreateCompatibleDC, DeleteDC, DeleteObject, GetDC,
        GetDIBits, ReleaseDC, SelectObject, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS,
        SRCCOPY,
    };
    use windows::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, GetWindowRect, IsIconic};

    // SAFETY: plain Win32 GDI calls on handles created and released in this
    // function; the pixel buffer is sized for the bitmap it receives.
    unsafe {
        let hwnd = GetForegroundWindow();
        if hwnd.0.is_null() || IsIconic(hwnd).as_bool() {
            return None;
        }

        // The visible frame, without the invisible resize border that
        // GetWindowRect includes on Windows 10 and later.
        let mut rect = RECT::default();
        let have_frame = DwmGetWindowAttribute(
            hwnd,
            DWMWA_EXTENDED_FRAME_BOUNDS,
            &mut rect as *mut RECT as *mut c_void,
            size_of::<RECT>() as u32,
        )
        .is_ok();
        if !have_frame && GetWindowRect(hwnd, &mut rect).is_err() {
            return None;
        }
        let width = rect.right - rect.left;
        let height = rect.bottom - rect.top;
        if width <= 0 || height <= 0 {
            return None;
        }

        let screen_dc = GetDC(None);
        if screen_dc.is_invalid() {
            return None;
        }
        let memory_dc = CreateCompatibleDC(Some(screen_dc));
        let bitmap = CreateCompatibleBitmap(screen_dc, width, height);
        let previous = SelectObject(memory_dc, bitmap.into());

        // Copies what is on screen, so a window covering part of the target
        // appears in the picture too.
        let copied = BitBlt(
            memory_dc,
            0,
            0,
            width,
            height,
            Some(screen_dc),
            rect.left,
            rect.top,
            SRCCOPY,
        )
        .is_ok();

        let mut info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: width,
                // Negative: rows run top to bottom.
                biHeight: -height,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut pixels = vec![0u8; width as usize * height as usize * 4];
        let rows = if copied {
            GetDIBits(
                memory_dc,
                bitmap,
                0,
                height as u32,
                Some(pixels.as_mut_ptr() as *mut c_void),
                &mut info,
                DIB_RGB_COLORS,
            )
        } else {
            0
        };

        SelectObject(memory_dc, previous);
        let _ = DeleteObject(bitmap.into());
        let _ = DeleteDC(memory_dc);
        ReleaseDC(None, screen_dc);

        if rows == 0 {
            warn!("Couldn't copy the active window for the bug report screenshot");
            return None;
        }

        // GDI gives BGRA with an unset alpha channel.
        for pixel in pixels.chunks_mut(4) {
            pixel.swap(0, 2);
            pixel[3] = 255;
        }
        Some(Shot {
            width: width as u32,
            height: height as u32,
            rgba: pixels,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_a_png_that_decodes_back_to_the_same_pixels() {
        let shot = Shot {
            width: 2,
            height: 1,
            rgba: vec![255, 0, 0, 255, 0, 0, 255, 255],
        };
        let data = encode_png(&shot).unwrap();
        assert_eq!(&data[..8], b"\x89PNG\r\n\x1a\n");

        let decoder = png::Decoder::new(std::io::Cursor::new(data));
        let mut reader = decoder.read_info().unwrap();
        let mut buf = vec![0; reader.output_buffer_size()];
        let info = reader.next_frame(&mut buf).unwrap();
        assert_eq!((info.width, info.height), (2, 1));
        assert_eq!(&buf[..8], &shot.rgba[..]);
    }

    /// Manual: needs a visible foreground window. Writes the capture to the
    /// path in `HANDY_SCREENSHOT_TEST_OUT`.
    #[test]
    #[ignore = "needs a desktop session with a foreground window"]
    fn captures_the_foreground_window() {
        let shot = capture_foreground().expect("a foreground window");
        assert!(shot.width > 0 && shot.height > 0);
        assert_eq!(
            shot.rgba.len(),
            shot.width as usize * shot.height as usize * 4
        );
        if let Ok(path) = std::env::var("HANDY_SCREENSHOT_TEST_OUT") {
            std::fs::write(path, encode_png(&shot).unwrap()).unwrap();
        }
    }

    #[test]
    fn a_capture_is_taken_once_and_a_new_start_replaces_it() {
        *pending() = Some(Shot {
            width: 1,
            height: 1,
            rgba: vec![0, 0, 0, 255],
        });
        assert!(take().is_some());
        assert!(take().is_none());
    }
}
