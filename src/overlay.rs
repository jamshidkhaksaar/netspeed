use crate::anchor::{Edge, TaskbarInfo};
use windows::core::{w, Free, Interface, PCWSTR};
use windows::Win32::Foundation::{COLORREF, HINSTANCE, HWND, POINT, RECT, SIZE};
use windows::Win32::Graphics::Direct2D::Common::{
    D2D1_ALPHA_MODE_PREMULTIPLIED, D2D1_COLOR_F, D2D1_PIXEL_FORMAT, D2D_RECT_F,
};
use windows::Win32::Graphics::Direct2D::{
    D2D1CreateFactory, D2D1_DRAW_TEXT_OPTIONS_NONE, D2D1_FACTORY_TYPE_SINGLE_THREADED,
    D2D1_FEATURE_LEVEL_DEFAULT, D2D1_RENDER_TARGET_PROPERTIES, D2D1_RENDER_TARGET_TYPE_DEFAULT,
    D2D1_RENDER_TARGET_USAGE_NONE, D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE, ID2D1Brush,
    ID2D1DCRenderTarget, ID2D1Factory,
};
use windows::Win32::Graphics::DirectWrite::{
    DWriteCreateFactory, DWRITE_FACTORY_TYPE_SHARED, DWRITE_FONT_STRETCH_NORMAL,
    DWRITE_FONT_STYLE_NORMAL, DWRITE_FONT_WEIGHT_NORMAL, DWRITE_MEASURING_MODE_NATURAL,
    DWRITE_PARAGRAPH_ALIGNMENT_CENTER, DWRITE_TEXT_ALIGNMENT_LEADING,
    DWRITE_TEXT_ALIGNMENT_TRAILING, DWRITE_TEXT_METRICS, DWRITE_WORD_WRAPPING_NO_WRAP,
    IDWriteFactory, IDWriteTextFormat,
};
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM;
use windows::Win32::Graphics::Gdi::{
    CreateCompatibleDC, CreateDIBSection, DeleteDC, ScreenToClient, SelectObject, AC_SRC_ALPHA,
    AC_SRC_OVER, BI_RGB, BITMAPINFO, BITMAPINFOHEADER, BLENDFUNCTION, DIB_RGB_COLORS, HBITMAP,
    HDC, HGDIOBJ,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DestroyWindow, SetWindowPos, ShowWindow, UpdateLayeredWindow, SW_HIDE,
    SW_SHOWNOACTIVATE, SWP_NOACTIVATE, ULW_ALPHA, WS_CHILD, WS_EX_LAYERED, WS_EX_NOACTIVATE,
    WS_EX_TOOLWINDOW, WS_EX_TRANSPARENT,
};
use windows_numerics::Matrix3x2;

pub const CLASS: PCWSTR = w!("NetSpeedOverlay");

const FONT_LOGICAL: f32 = 12.0;

pub struct Overlay {
    pub hwnd: HWND,
    hdc: HDC,
    old_bmp: HGDIOBJ,
    hbmp: Option<HBITMAP>,
    bits: *mut core::ffi::c_void,
    target: ID2D1DCRenderTarget,
    dwrite: IDWriteFactory,
    fmt_arrow: IDWriteTextFormat,
    fmt_value: IDWriteTextFormat,
    fmt_measure: IDWriteTextFormat,
    brush_text: ID2D1Brush,
    brush_down: ID2D1Brush,
    brush_up: ID2D1Brush,
    dark: bool,
    dpi: u32,
    font_px: f32,
    arrow_w: i32,
    canvas_w: i32,
    canvas_w_cur: i32,
    line_h: i32,
    canvas_h: i32,
    transform: Matrix3x2,
    win_w: i32,
    win_h: i32,
    last_placed: RECT,
    last_place_key: Option<(i32, i32, i32, i32, Edge)>,
    visible: bool,
    shown: bool,
}

pub fn font_px(dpi: u32) -> f32 {
    FONT_LOGICAL * (dpi as f32).clamp(72.0, 384.0) / 96.0
}

fn zero_rect() -> RECT {
    RECT {
        left: 0,
        top: 0,
        right: 0,
        bottom: 0,
    }
}

fn same_rect(a: &RECT, b: &RECT) -> bool {
    a.left == b.left && a.top == b.top && a.right == b.right && a.bottom == b.bottom
}

fn color_text(dark: bool) -> D2D1_COLOR_F {
    if dark {
        D2D1_COLOR_F {
            r: 1.0,
            g: 1.0,
            b: 1.0,
            a: 1.0,
        }
    } else {
        D2D1_COLOR_F {
            r: 0.1059,
            g: 0.1059,
            b: 0.1059,
            a: 1.0,
        }
    }
}

fn color_down(dark: bool) -> D2D1_COLOR_F {
    if dark {
        D2D1_COLOR_F {
            r: 0.4235,
            g: 0.7961,
            b: 0.3725,
            a: 1.0,
        }
    } else {
        D2D1_COLOR_F {
            r: 0.0588,
            g: 0.4824,
            b: 0.0588,
            a: 1.0,
        }
    }
}

fn color_up(dark: bool) -> D2D1_COLOR_F {
    if dark {
        D2D1_COLOR_F {
            r: 0.3765,
            g: 0.8039,
            b: 1.0,
            a: 1.0,
        }
    } else {
        D2D1_COLOR_F {
            r: 0.0,
            g: 0.3725,
            b: 0.7216,
            a: 1.0,
        }
    }
}

unsafe fn make_brush(rt: &ID2D1DCRenderTarget, c: &D2D1_COLOR_F) -> Option<ID2D1Brush> {
    rt.CreateSolidColorBrush(c, None)
        .ok()
        .and_then(|b| b.cast::<ID2D1Brush>().ok())
}

unsafe fn measure(dwrite: &IDWriteFactory, fmt: &IDWriteTextFormat, s: &str) -> i32 {
    let wide: Vec<u16> = s.encode_utf16().collect();
    let layout = match dwrite.CreateTextLayout(&wide, fmt, 4096.0, 128.0) {
        Ok(l) => l,
        Err(_) => return 20,
    };
    let mut m = DWRITE_TEXT_METRICS::default();
    if layout.GetMetrics(&mut m).is_err() {
        return 20;
    }
    m.width.ceil() as i32
}

unsafe fn draw_text(
    rt: &ID2D1DCRenderTarget,
    fmt: &IDWriteTextFormat,
    brush: &ID2D1Brush,
    s: &str,
    rect: &D2D_RECT_F,
) {
    let wide: Vec<u16> = s.encode_utf16().collect();
    rt.DrawText(
        &wide,
        fmt,
        rect,
        brush,
        D2D1_DRAW_TEXT_OPTIONS_NONE,
        DWRITE_MEASURING_MODE_NATURAL,
    );
}

impl Overlay {
    pub fn create(tb: &TaskbarInfo, dark: bool, dpi: u32) -> Option<Overlay> {
        unsafe {
            let hmodule = match GetModuleHandleW(PCWSTR::null()) {
                Ok(h) => h,
                Err(_) => {
                    crate::util::log_error("overlay: GetModuleHandleW failed");
                    return None;
                }
            };
            let inst = HINSTANCE(hmodule.0);
            let hwnd = match CreateWindowExW(
                WS_EX_LAYERED | WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW | WS_EX_TRANSPARENT,
                CLASS,
                w!(""),
                WS_CHILD,
                -32000,
                -32000,
                10,
                10,
                Some(tb.hwnd),
                None,
                Some(inst),
                None,
            ) {
                Ok(h) => h,
                Err(e) => {
                    let le = windows::Win32::Foundation::GetLastError();
                    crate::util::log_error(&format!(
                        "overlay: CreateWindowExW failed: {e} (last=0x{:X})",
                        le.0
                    ));
                    return None;
                }
            };
            match Self::build(hwnd, dark, dpi) {
                Some(o) => Some(o),
                None => {
                    let _ = DestroyWindow(hwnd);
                    None
                }
            }
        }
    }

    unsafe fn build(hwnd: HWND, dark: bool, dpi: u32) -> Option<Overlay> {
        let hdc = CreateCompatibleDC(None);
        if hdc.0.is_null() {
            crate::util::log_error("overlay: CreateCompatibleDC failed");
            return None;
        }
        let target = match Self::make_target() {
            Some(t) => t,
            None => {
                let _ = DeleteDC(hdc);
                return None;
            }
        };
        let (dwrite, fmt_arrow, fmt_value, fmt_measure) = match Self::make_formats(dpi) {
            Some(f) => f,
            None => {
                crate::util::log_error("overlay: DirectWrite format creation failed");
                let _ = DeleteDC(hdc);
                return None;
            }
        };
        let (bt, bd, bu) = match (
            make_brush(&target, &color_text(dark)),
            make_brush(&target, &color_down(dark)),
            make_brush(&target, &color_up(dark)),
        ) {
            (Some(a), Some(b), Some(c)) => (a, b, c),
            _ => {
                crate::util::log_error("overlay: solid brush creation failed");
                let _ = DeleteDC(hdc);
                return None;
            }
        };
        let _ = target.SetTextAntialiasMode(D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE);
        let arrow_w = measure(&dwrite, &fmt_measure, "\u{2193}").max(6);
        let text_w = measure(&dwrite, &fmt_measure, "888.88 MiB/s").max(40);
        let canvas_w = 3 + arrow_w + 3 + text_w + 4;
        Some(Overlay {
            hwnd,
            hdc,
            old_bmp: HGDIOBJ(std::ptr::null_mut()),
            hbmp: None,
            bits: std::ptr::null_mut(),
            target,
            dwrite,
            fmt_arrow,
            fmt_value,
            fmt_measure,
            brush_text: bt,
            brush_down: bd,
            brush_up: bu,
            dark,
            dpi,
            font_px: font_px(dpi),
            arrow_w,
            canvas_w,
            canvas_w_cur: canvas_w,
            line_h: 16,
            canvas_h: 32,
            transform: Matrix3x2 {
                M11: 1.0,
                M12: 0.0,
                M21: 0.0,
                M22: 1.0,
                M31: 0.0,
                M32: 0.0,
            },
            win_w: 0,
            win_h: 0,
            last_placed: zero_rect(),
            last_place_key: None,
            visible: true,
            shown: false,
        })
    }

    unsafe fn make_target() -> Option<ID2D1DCRenderTarget> {
        let factory: ID2D1Factory = match D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None)
        {
            Ok(f) => f,
            Err(_) => {
                crate::util::log_error("overlay: D2D1CreateFactory failed");
                return None;
            }
        };
        let props = D2D1_RENDER_TARGET_PROPERTIES {
            r#type: D2D1_RENDER_TARGET_TYPE_DEFAULT,
            pixelFormat: D2D1_PIXEL_FORMAT {
                format: DXGI_FORMAT_B8G8R8A8_UNORM,
                alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
            },
            dpiX: 96.0,
            dpiY: 96.0,
            usage: D2D1_RENDER_TARGET_USAGE_NONE,
            minLevel: D2D1_FEATURE_LEVEL_DEFAULT,
        };
        match factory.CreateDCRenderTarget(&props) {
            Ok(t) => Some(t),
            Err(_) => {
                crate::util::log_error("overlay: CreateDCRenderTarget failed");
                None
            }
        }
    }

    unsafe fn make_formats(
        dpi: u32,
    ) -> Option<(
        IDWriteFactory,
        IDWriteTextFormat,
        IDWriteTextFormat,
        IDWriteTextFormat,
    )> {
        let dwrite: IDWriteFactory = match DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED) {
            Ok(f) => f,
            Err(_) => {
                crate::util::log_error("overlay: DWriteCreateFactory failed");
                return None;
            }
        };
        let px = font_px(dpi);
        let fam = w!("Segoe UI Variable");
        let alt = w!("Segoe UI");
        let mk = |ff: PCWSTR| -> Option<IDWriteTextFormat> {
            dwrite
                .CreateTextFormat(
                    ff,
                    None,
                    DWRITE_FONT_WEIGHT_NORMAL,
                    DWRITE_FONT_STYLE_NORMAL,
                    DWRITE_FONT_STRETCH_NORMAL,
                    px,
                    w!("en-us"),
                )
                .ok()
        };
        let fmt_arrow = mk(fam).or_else(|| mk(alt))?;
        let _ = fmt_arrow.SetTextAlignment(DWRITE_TEXT_ALIGNMENT_LEADING);
        let _ = fmt_arrow.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_CENTER);
        let _ = fmt_arrow.SetWordWrapping(DWRITE_WORD_WRAPPING_NO_WRAP);
        let fmt_value = mk(fam).or_else(|| mk(alt))?;
        let _ = fmt_value.SetTextAlignment(DWRITE_TEXT_ALIGNMENT_TRAILING);
        let _ = fmt_value.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_CENTER);
        let _ = fmt_value.SetWordWrapping(DWRITE_WORD_WRAPPING_NO_WRAP);
        let fmt_measure = mk(fam).or_else(|| mk(alt))?;
        let _ = fmt_measure.SetTextAlignment(DWRITE_TEXT_ALIGNMENT_LEADING);
        let _ = fmt_measure.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_CENTER);
        let _ = fmt_measure.SetWordWrapping(DWRITE_WORD_WRAPPING_NO_WRAP);
        Some((dwrite, fmt_arrow, fmt_value, fmt_measure))
    }

    pub fn place(&mut self, tb: &TaskbarInfo) -> bool {
        let a = tb.anchor;
        let vertical = matches!(tb.edge, Edge::Left | Edge::Right);
        let thickness = if vertical {
            tb.rect.right - tb.rect.left
        } else {
            tb.rect.bottom - tb.rect.top
        };
        let line_h = ((self.font_px * 1.35).ceil() as i32).max(10);
        let canvas_h = (line_h * 2).min((thickness - 2).max(16));
        let avail = if vertical {
            a.top - tb.rect.top - 8
        } else {
            a.left - tb.rect.left - 8
        };
        let canvas_w_cur = self.canvas_w.min(avail.max(24));
        self.line_h = line_h;
        self.canvas_h = canvas_h;
        self.canvas_w_cur = canvas_w_cur;
        let (win_w, win_h) = if vertical {
            (canvas_h, canvas_w_cur)
        } else {
            (canvas_w_cur, canvas_h)
        };
        let key = (win_w, win_h, canvas_w_cur, canvas_h, tb.edge);
        let changed = self.last_place_key != Some(key);
        if vertical {
            self.transform = if tb.edge == Edge::Left {
                Matrix3x2 {
                    M11: 0.0,
                    M12: -1.0,
                    M21: 1.0,
                    M22: 0.0,
                    M31: 0.0,
                    M32: canvas_w_cur as f32,
                }
            } else {
                Matrix3x2 {
                    M11: 0.0,
                    M12: 1.0,
                    M21: -1.0,
                    M22: 0.0,
                    M31: canvas_h as f32,
                    M32: 0.0,
                }
            };
        } else {
            self.transform = Matrix3x2 {
                M11: 1.0,
                M12: 0.0,
                M21: 0.0,
                M22: 1.0,
                M31: 0.0,
                M32: 0.0,
            };
        }
        let mut size_ok = true;
        if self.win_w != win_w || self.win_h != win_h {
            size_ok = unsafe { self.make_dib(win_w, win_h) };
        }
        if !size_ok {
            return false;
        }
        self.win_w = win_w;
        self.win_h = win_h;
        self.last_place_key = Some(key);
        let (sx, sy) = if vertical {
            (
                tb.rect.left + (thickness - canvas_h) / 2,
                (a.top - canvas_w_cur - 4).max(tb.rect.top + 2),
            )
        } else {
            (
                (a.left - canvas_w_cur - 4).max(tb.rect.left + 2),
                tb.rect.top + (thickness - canvas_h) / 2,
            )
        };
        let mut pt = POINT { x: sx, y: sy };
        unsafe {
            let _ = ScreenToClient(tb.hwnd, &mut pt);
        }
        let r = RECT {
            left: pt.x,
            top: pt.y,
            right: pt.x + win_w,
            bottom: pt.y + win_h,
        };
        if !same_rect(&r, &self.last_placed) {
            unsafe {
                let _ = SetWindowPos(self.hwnd, None, r.left, r.top, win_w, win_h, SWP_NOACTIVATE);
            }
            self.last_placed = r;
        }
        if self.visible && !self.shown {
            unsafe {
                let _ = ShowWindow(self.hwnd, SW_SHOWNOACTIVATE);
            }
            self.shown = true;
        }
        changed
    }

    unsafe fn make_dib(&mut self, w: i32, h: i32) -> bool {
        if let Some(old) = self.hbmp.take() {
            let _ = SelectObject(self.hdc, self.old_bmp);
            let mut ob = old;
            ob.free();
        }
        let mut bmi = BITMAPINFO::default();
        bmi.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
        bmi.bmiHeader.biWidth = w;
        bmi.bmiHeader.biHeight = -h;
        bmi.bmiHeader.biPlanes = 1;
        bmi.bmiHeader.biBitCount = 32;
        bmi.bmiHeader.biCompression = BI_RGB.0;
        let mut bits: *mut core::ffi::c_void = std::ptr::null_mut();
        let hbmp = match CreateDIBSection(None, &bmi, DIB_RGB_COLORS, &mut bits, None, 0) {
            Ok(b) => b,
            Err(_) => return false,
        };
        let old = SelectObject(self.hdc, HGDIOBJ(hbmp.0));
        if self.old_bmp.0.is_null() {
            self.old_bmp = old;
        }
        self.hbmp = Some(hbmp);
        self.bits = bits;
        let r = RECT {
            left: 0,
            top: 0,
            right: w,
            bottom: h,
        };
        let _ = self.target.BindDC(self.hdc, &r);
        true
    }

    pub fn snap_bmp(&self, path: &str) -> bool {
        let (w, h) = (self.win_w, self.win_h);
        if self.bits.is_null() || w <= 0 || h <= 0 {
            return false;
        }
        let px = (w as usize) * (h as usize) * 4;
        let src = unsafe { std::slice::from_raw_parts(self.bits as *const u8, px) };
        let mut out = Vec::with_capacity(54 + px);
        out.extend_from_slice(&0x4D42u16.to_le_bytes());
        out.extend_from_slice(&((54 + px) as u32).to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&54u32.to_le_bytes());
        out.extend_from_slice(&40u32.to_le_bytes());
        out.extend_from_slice(&w.to_le_bytes());
        out.extend_from_slice(&(-h).to_le_bytes());
        out.extend_from_slice(&1u16.to_le_bytes());
        out.extend_from_slice(&32u16.to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
        out.extend_from_slice(&(px as u32).to_le_bytes());
        out.extend_from_slice(&2835u32.to_le_bytes());
        out.extend_from_slice(&2835u32.to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
        out.extend_from_slice(src);
        std::fs::write(path, out).is_ok()
    }

    pub fn render(&mut self, down: &str, up: &str) {
        if self.win_w <= 0 || self.win_h <= 0 {
            return;
        }
        unsafe {
            let rt = &self.target;
            rt.BeginDraw();
            let transparent = D2D1_COLOR_F {
                r: 0.0,
                g: 0.0,
                b: 0.0,
                a: 0.0,
            };
            rt.Clear(Some(&transparent));
            let _ = rt.SetTransform(&self.transform);
            let lh = self.line_h as f32;
            let aw = self.arrow_w as f32;
            let cw = self.canvas_w_cur as f32;
            let d1 = D2D_RECT_F {
                left: 0.0,
                top: 0.0,
                right: aw,
                bottom: lh,
            };
            draw_text(rt, &self.fmt_arrow, &self.brush_down, "\u{2193}", &d1);
            let v1 = D2D_RECT_F {
                left: aw + 3.0,
                top: 0.0,
                right: cw - 4.0,
                bottom: lh,
            };
            draw_text(rt, &self.fmt_value, &self.brush_text, down, &v1);
            let d2 = D2D_RECT_F {
                left: 0.0,
                top: lh,
                right: aw,
                bottom: lh * 2.0,
            };
            draw_text(rt, &self.fmt_arrow, &self.brush_up, "\u{2191}", &d2);
            let v2 = D2D_RECT_F {
                left: aw + 3.0,
                top: lh,
                right: cw - 4.0,
                bottom: lh * 2.0,
            };
            draw_text(rt, &self.fmt_value, &self.brush_text, up, &v2);
            let _ = rt.EndDraw(None, None);
            let size = SIZE {
                cx: self.win_w,
                cy: self.win_h,
            };
            let src = POINT { x: 0, y: 0 };
            let blend = BLENDFUNCTION {
                BlendOp: AC_SRC_OVER as u8,
                BlendFlags: 0,
                SourceConstantAlpha: 255,
                AlphaFormat: AC_SRC_ALPHA as u8,
            };
            let _ = UpdateLayeredWindow(
                self.hwnd,
                None,
                None,
                Some(&size),
                Some(self.hdc),
                Some(&src),
                COLORREF(0),
                Some(&blend),
                ULW_ALPHA,
            );
        }
    }

    pub fn set_dark(&mut self, dark: bool) {
        if self.dark == dark {
            return;
        }
        self.dark = dark;
        unsafe {
            if let Some(b) = make_brush(&self.target, &color_text(dark)) {
                self.brush_text = b;
            }
            if let Some(b) = make_brush(&self.target, &color_down(dark)) {
                self.brush_down = b;
            }
            if let Some(b) = make_brush(&self.target, &color_up(dark)) {
                self.brush_up = b;
            }
        }
    }

    pub fn update_dpi(&mut self, dpi: u32) {
        if dpi == self.dpi {
            return;
        }
        unsafe {
            if let Some((dw, fa, fv, fm)) = Self::make_formats(dpi) {
                self.dwrite = dw;
                self.fmt_arrow = fa;
                self.fmt_value = fv;
                self.fmt_measure = fm;
                self.dpi = dpi;
                self.font_px = font_px(dpi);
                self.arrow_w = measure(&self.dwrite, &self.fmt_measure, "\u{2193}").max(6);
                let tw = measure(&self.dwrite, &self.fmt_measure, "888.88 MiB/s").max(40);
                self.canvas_w = 3 + self.arrow_w + 3 + tw + 4;
            }
        }
    }

    pub fn set_visible(&mut self, v: bool) {
        self.visible = v;
        unsafe {
            let _ = ShowWindow(self.hwnd, if v { SW_SHOWNOACTIVATE } else { SW_HIDE });
        }
        self.shown = v;
    }
}

impl Drop for Overlay {
    fn drop(&mut self) {
        unsafe {
            if let Some(mut b) = self.hbmp.take() {
                let _ = SelectObject(self.hdc, self.old_bmp);
                b.free();
            }
            let _ = DeleteDC(self.hdc);
            let _ = DestroyWindow(self.hwnd);
        }
    }
}
