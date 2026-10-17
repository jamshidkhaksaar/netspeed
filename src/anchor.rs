use windows::core::{w, BOOL, PCWSTR};
use windows::Win32::Foundation::{HWND, LPARAM, RECT};
use windows::Win32::Graphics::Gdi::{
    GetMonitorInfoW, MonitorFromWindow, MONITORINFO, MONITOR_DEFAULTTONEAREST,
};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumChildWindows, FindWindowW, GetClassNameW, GetWindowRect, IsWindowVisible,
};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Edge {
    Top,
    Bottom,
    Left,
    Right,
}

#[derive(Clone, Debug)]
pub struct TaskbarInfo {
    pub hwnd: HWND,
    pub rect: RECT,
    pub edge: Edge,
    pub anchor: RECT,
}

const MAX_CHILDREN: usize = 64;

fn zero_rect() -> RECT {
    RECT {
        left: 0,
        top: 0,
        right: 0,
        bottom: 0,
    }
}

struct ChildEnum {
    count: usize,
    classes: [String; MAX_CHILDREN],
    rects: [RECT; MAX_CHILDREN],
    visible: [bool; MAX_CHILDREN],
}

impl ChildEnum {
    fn new() -> Self {
        ChildEnum {
            count: 0,
            classes: std::array::from_fn(|_| String::new()),
            rects: [zero_rect(); MAX_CHILDREN],
            visible: [false; MAX_CHILDREN],
        }
    }
}

unsafe extern "system" fn enum_cb(hwnd: HWND, lparam: LPARAM) -> BOOL {
    let e = &mut *(lparam.0 as *mut ChildEnum);
    if e.count < MAX_CHILDREN {
        let i = e.count;
        let mut buf = [0u16; 64];
        let n = GetClassNameW(hwnd, &mut buf) as usize;
        e.classes[i] = if n > 0 {
            String::from_utf16_lossy(&buf[..n])
        } else {
            String::new()
        };
        let mut r = zero_rect();
        let _ = GetWindowRect(hwnd, &mut r);
        e.rects[i] = r;
        e.visible[i] = IsWindowVisible(hwnd).as_bool();
        e.count += 1;
    }
    BOOL(1)
}

pub fn find_taskbar() -> Option<TaskbarInfo> {
    unsafe {
        let hwnd = FindWindowW(w!("Shell_TrayWnd"), PCWSTR::null()).ok()?;
        if hwnd.0.is_null() {
            return None;
        }
        let mut rect = zero_rect();
        if GetWindowRect(hwnd, &mut rect).is_err() {
            return None;
        }
        let edge = detect_edge(hwnd, &rect)?;
        let mut e = ChildEnum::new();
        let _ = EnumChildWindows(Some(hwnd), Some(enum_cb), LPARAM(&mut e as *mut _ as isize));
        let anchor = choose_anchor(&e, &rect).unwrap_or_else(|| geometric_anchor(&rect));
        Some(TaskbarInfo {
            hwnd,
            rect,
            edge,
            anchor,
        })
    }
}

unsafe fn detect_edge(hwnd: HWND, tb: &RECT) -> Option<Edge> {
    let mon = MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST);
    let mut mi = MONITORINFO {
        cbSize: std::mem::size_of::<MONITORINFO>() as u32,
        rcMonitor: zero_rect(),
        rcWork: zero_rect(),
        dwFlags: 0,
    };
    if !GetMonitorInfoW(mon, &mut mi).as_bool() {
        return None;
    }
    let m = mi.rcMonitor;
    let dw = (m.right - m.left).max(1);
    let dh = (m.bottom - m.top).max(1);
    let w = tb.right - tb.left;
    let h = tb.bottom - tb.top;
    if w >= (dw * 8) / 10 && (tb.top - m.top).abs() <= 4 {
        Some(Edge::Top)
    } else if w >= (dw * 8) / 10 && (m.bottom - tb.bottom).abs() <= 4 {
        Some(Edge::Bottom)
    } else if h >= (dh * 8) / 10 && (tb.left - m.left).abs() <= 4 {
        Some(Edge::Left)
    } else if h >= (dh * 8) / 10 && (m.right - tb.right).abs() <= 4 {
        Some(Edge::Right)
    } else {
        None
    }
}

fn plausible(r: &RECT, tb: &RECT) -> bool {
    let w = r.right - r.left;
    let h = r.bottom - r.top;
    if w < 8 || h < 8 {
        return false;
    }
    let tb_span = (tb.right - tb.left).max(tb.bottom - tb.top);
    w.max(h) < (tb_span * 55) / 100
}

fn choose_anchor(e: &ChildEnum, tb: &RECT) -> Option<RECT> {
    let mut clock = None;
    let mut notify = None;
    for i in 0..e.count {
        if !e.visible[i] {
            continue;
        }
        let r = e.rects[i];
        if r.right <= r.left || r.bottom <= r.top {
            continue;
        }
        match e.classes[i].as_str() {
            "TrayClockWClass" => clock = Some(r),
            "TrayNotifyWnd" => notify = Some(r),
            _ => {}
        }
    }
    for cand in [clock, notify].into_iter().flatten() {
        if plausible(&cand, tb) {
            return Some(cand);
        }
    }
    cluster_anchor(e, tb)
}

fn cluster_anchor(e: &ChildEnum, tb: &RECT) -> Option<RECT> {
    let w = tb.right - tb.left;
    let h = tb.bottom - tb.top;
    let vertical = h > w;
    let min_extent = if vertical { h } else { w };
    let origin = if vertical { tb.top } else { tb.left };
    let cutoff = origin + (min_extent * 60) / 100;
    let mut best_start = i32::MAX;
    let mut best: Option<RECT> = None;
    for i in 0..e.count {
        if !e.visible[i] {
            continue;
        }
        let r = e.rects[i];
        if r.right <= r.left || r.bottom <= r.top {
            continue;
        }
        let start = if vertical { r.top } else { r.left };
        if start < cutoff {
            continue;
        }
        if start < best_start {
            best_start = start;
            best = Some(r);
        }
    }
    best.map(|r| {
        if vertical {
            RECT {
                left: tb.left,
                top: r.top,
                right: tb.right,
                bottom: r.top + 24,
            }
        } else {
            RECT {
                left: r.left,
                top: tb.top,
                right: r.left + 24,
                bottom: tb.bottom,
            }
        }
    })
}

fn geometric_anchor(tb: &RECT) -> RECT {
    let w = tb.right - tb.left;
    let h = tb.bottom - tb.top;
    if h > w {
        let y = tb.top + (h * 3) / 4;
        RECT {
            left: tb.left,
            top: y,
            right: tb.right,
            bottom: y + 24,
        }
    } else {
        let x = tb.left + (w * 3) / 4;
        RECT {
            left: x,
            top: tb.top,
            right: x + 24,
            bottom: tb.bottom,
        }
    }
}

pub fn debug_dump() {
    unsafe {
        match FindWindowW(w!("Shell_TrayWnd"), PCWSTR::null()) {
            Ok(hwnd) if !hwnd.0.is_null() => {
                let mut r = zero_rect();
                let _ = GetWindowRect(hwnd, &mut r);
                println!(
                    "Shell_TrayWnd hwnd=0x{:X} rect=({},{},{},{}) edge={:?}",
                    hwnd.0 as isize,
                    r.left,
                    r.top,
                    r.right,
                    r.bottom,
                    detect_edge(hwnd, &r)
                );
                println!(
                    "dpi: taskbar={} self_thread_ctx={} taskbar_ctx={}",
                    windows::Win32::UI::HiDpi::GetDpiForWindow(hwnd),
                    windows::Win32::UI::HiDpi::GetThreadDpiAwarenessContext().0 as isize,
                    windows::Win32::UI::HiDpi::GetWindowDpiAwarenessContext(hwnd).0 as isize
                );
                let mut e = ChildEnum::new();
                let _ =
                    EnumChildWindows(Some(hwnd), Some(enum_cb), LPARAM(&mut e as *mut _ as isize));
                for i in 0..e.count {
                    let cr = e.rects[i];
                    println!(
                        "  [{}] {:36} rect=({:5},{:5},{:5},{:5}) vis={} plausible={}",
                        i,
                        e.classes[i],
                        cr.left,
                        cr.top,
                        cr.right,
                        cr.bottom,
                        e.visible[i],
                        plausible(&cr, &r)
                    );
                }
                match find_taskbar() {
                    Some(tb) => println!(
                        "chosen anchor=({},{},{},{})",
                        tb.anchor.left, tb.anchor.top, tb.anchor.right, tb.anchor.bottom
                    ),
                    None => println!("no anchor chosen"),
                }
            }
            _ => println!("Shell_TrayWnd not found"),
        }
    }
}
