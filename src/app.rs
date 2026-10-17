use crate::anchor::{self, TaskbarInfo};
use crate::config::{Config, Units};
use crate::format;
use crate::log::Logger;
use crate::net::{self, AdapterRow};
use crate::overlay::Overlay;
use crate::theme;
use crate::tray::{self, Tray};
use std::time::{Duration, Instant};
use windows::core::PCWSTR;
use windows::Win32::Foundation::{ERROR_SUCCESS, HWND};
use windows::Win32::System::Registry::{
    RegCloseKey, RegDeleteValueW, RegOpenKeyExW, RegSetValueExW, HKEY, HKEY_CURRENT_USER,
    KEY_SET_VALUE, REG_SZ,
};
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::WindowsAndMessaging::{KillTimer, SetTimer};

const RUN_KEY: &str = "Software\\Microsoft\\Windows\\CurrentVersion\\Run";
const RUN_VALUE: &str = "NetSpeed";

pub struct App {
    pub cfg: Config,
    pub tray: Tray,
    overlay: Option<Overlay>,
    overlay_retry_at: Instant,
    taskbar_hwnd: HWND,
    paused: bool,
    adapters: Vec<AdapterRow>,
    selected_name: String,
    prev_in: u64,
    prev_out: u64,
    reset_pending: bool,
    last_sample: Instant,
    tot_in: u64,
    tot_out: u64,
    rate_down: f64,
    rate_up: f64,
    last_text: Option<(String, String)>,
    logger: Option<Logger>,
    logger_at: Instant,
    theme_dark: bool,
    theme_at: Instant,
    tip_at: Instant,
    taskbar_created_msg: u32,
    pub snap_path: Option<String>,
    snap_ticks: u32,
}

impl App {
    pub fn new(cfg: Config, tray_hwnd: HWND, taskbar_created_msg: u32) -> App {
        let now = Instant::now();
        App {
            cfg,
            tray: Tray::new(tray_hwnd),
            overlay: None,
            overlay_retry_at: now,
            taskbar_hwnd: HWND::default(),
            paused: false,
            adapters: Vec::new(),
            selected_name: String::from("--"),
            prev_in: 0,
            prev_out: 0,
            reset_pending: true,
            last_sample: now,
            tot_in: 0,
            tot_out: 0,
            rate_down: 0.0,
            rate_up: 0.0,
            last_text: None,
            logger: None,
            logger_at: now,
            theme_dark: theme::is_dark(),
            theme_at: now,
            tip_at: now - Duration::from_secs(10),
            taskbar_created_msg,
            snap_path: None,
            snap_ticks: 0,
        }
    }

    pub fn init(&mut self) {
        set_autostart(self.cfg.autostart);
        self.tray.add_icon();
        self.tick();
    }

    pub fn start_timer(&self) {
        unsafe {
            SetTimer(
                Some(self.tray.hwnd),
                crate::TIMER_ID,
                self.cfg.interval_clamped(),
                None,
            );
        }
    }

    fn reset_timer(&self) {
        unsafe {
            let _ = KillTimer(Some(self.tray.hwnd), crate::TIMER_ID);
            SetTimer(
                Some(self.tray.hwnd),
                crate::TIMER_ID,
                self.cfg.interval_clamped(),
                None,
            );
        }
    }

    pub fn tray_hwnd(&self) -> HWND {
        self.tray.hwnd
    }

    pub fn taskbar_created_msg(&self) -> u32 {
        self.taskbar_created_msg
    }

    pub fn tick(&mut self) {
        let Some(tb) = anchor::find_taskbar() else {
            return;
        };
        if self.taskbar_hwnd != tb.hwnd {
            self.taskbar_hwnd = tb.hwnd;
            self.overlay = None;
            self.overlay_retry_at = Instant::now();
        }
        if self.overlay.is_none() && Instant::now() >= self.overlay_retry_at {
            self.overlay_retry_at = Instant::now() + Duration::from_secs(30);
            self.create_overlay_for(&tb);
        }
        if self.theme_at.elapsed() >= Duration::from_secs(5) {
            self.theme_at = Instant::now();
            let dark = theme::is_dark();
            if dark != self.theme_dark {
                self.theme_dark = dark;
                self.last_text = None;
            }
        }
        if let Some(o) = &mut self.overlay {
            let dpi = unsafe { GetDpiForWindow(tb.hwnd) }.max(96);
            o.update_dpi(dpi);
            o.set_dark(self.theme_dark);
            if o.place(&tb) {
                self.last_text = None;
            }
        }
        self.sample();
        self.render();
        if self.tip_at.elapsed() >= Duration::from_secs(2) {
            self.tip_at = Instant::now();
            self.update_tip();
        }
        self.logging_tick();
        if self.snap_path.is_some() {
            self.snap_ticks += 1;
            if self.snap_ticks >= 3 {
                let p = self.snap_path.clone().unwrap_or_default();
                if let Some(o) = &self.overlay {
                    o.snap_bmp(&p);
                }
                std::process::exit(0);
            }
        }
    }

    fn create_overlay_for(&mut self, tb: &TaskbarInfo) {
        self.overlay = None;
        let dark = self.theme_dark;
        let dpi = unsafe { GetDpiForWindow(tb.hwnd) }.max(96);
        match Overlay::create(tb, dark, dpi) {
            Some(mut o) => {
                o.set_visible(false);
                o.place(tb);
                o.set_visible(self.cfg.show);
                self.overlay = Some(o);
                self.last_text = None;
            }
            None => crate::util::log_error("overlay create failed"),
        }
    }

    fn sample(&mut self) {
        let now = Instant::now();
        self.adapters = net::list_adapters();
        if self.paused {
            self.last_sample = now;
            return;
        }
        let Some(row) = self.pick_adapter() else {
            return;
        };
        self.selected_name = if row.alias.is_empty() {
            row.description.clone()
        } else {
            row.alias.clone()
        };
        let dt = now.duration_since(self.last_sample).as_secs_f64();
        self.last_sample = now;
        let (i, o) = (row.in_octets, row.out_octets);
        if self.reset_pending {
            self.reset_pending = false;
            self.prev_in = i;
            self.prev_out = o;
            return;
        }
        if dt >= 0.05 && dt <= 30.0 {
            if i >= self.prev_in && o >= self.prev_out {
                let di = i - self.prev_in;
                let dout = o - self.prev_out;
                self.tot_in += di;
                self.tot_out += dout;
                self.rate_down = di as f64 / dt;
                self.rate_up = dout as f64 / dt;
            } else {
                self.rate_down = 0.0;
                self.rate_up = 0.0;
            }
        }
        self.prev_in = i;
        self.prev_out = o;
    }

    fn pick_adapter(&self) -> Option<AdapterRow> {
        if let Some(luid) = self.cfg.adapter_luid
            && let Some(r) = self.adapters.iter().find(|r| r.luid == luid)
        {
            return Some(r.clone());
        }
        if let Some(idx) = net::best_route_index()
            && let Some(r) = self.adapters.iter().find(|r| r.index == idx)
        {
            return Some(r.clone());
        }
        self.adapters
            .iter()
            .filter(|r| r.up && r.is_physical_like())
            .max_by_key(|r| r.link_bps)
            .cloned()
    }

    fn render(&mut self) {
        if let Some(o) = &mut self.overlay {
            let d = format::format_rate(self.rate_down, self.cfg.units, self.cfg.decimals);
            let u = format::format_rate(self.rate_up, self.cfg.units, self.cfg.decimals);
            if self.last_text.as_ref() == Some(&(d.clone(), u.clone())) {
                return;
            }
            o.render(&d, &u);
            self.last_text = Some((d, u));
        }
    }

    fn update_tip(&self) {
        let tip = if self.paused {
            format!("NetSpeed - paused\n{}", self.selected_name)
        } else {
            format!(
                "NetSpeed - {}\nD: {}  U: {}\nSession D: {}  U: {}",
                self.selected_name,
                format::format_rate(self.rate_down, self.cfg.units, self.cfg.decimals),
                format::format_rate(self.rate_up, self.cfg.units, self.cfg.decimals),
                format::format_bytes(self.tot_in),
                format::format_bytes(self.tot_out)
            )
        };
        self.tray.update_tip(&tip);
    }

    fn logging_tick(&mut self) {
        if !self.cfg.logging {
            self.logger = None;
            return;
        }
        let now = Instant::now();
        let secs = (self.cfg.interval_clamped() as u64).div_ceil(1000).max(1);
        if now.duration_since(self.logger_at) < Duration::from_secs(secs) {
            return;
        }
        self.logger_at = now;
        if self.logger.is_none() {
            self.logger = Some(Logger::new(self.cfg.log_retention_days));
        }
        if let Some(lg) = &mut self.logger {
            lg.log_sample(&self.selected_name, self.rate_down, self.rate_up);
        }
    }

    fn reset_sampling(&mut self) {
        self.reset_pending = true;
        self.last_sample = Instant::now();
    }

    pub fn toggle_show(&mut self) {
        self.cfg.show = !self.cfg.show;
        self.cfg.save();
        if let Some(o) = &mut self.overlay {
            o.set_visible(self.cfg.show);
        }
    }

    pub fn show_menu(&self) -> u32 {
        tray::show_menu(self.tray.hwnd, &self.cfg, &self.adapters, self.paused)
    }

    pub fn on_menu_command(&mut self, id: u32) -> bool {
        match id {
            tray::CMD_PAUSE => {
                self.paused = !self.paused;
                self.reset_sampling();
            }
            tray::CMD_SHOW => self.toggle_show(),
            tray::CMD_EXIT => return false,
            tray::CMD_OPEN_LOGS => {
                crate::util::open_path(&crate::util::data_dir().join("logs"));
            }
            tray::CMD_LOGGING => {
                self.cfg.logging = !self.cfg.logging;
                self.cfg.save();
            }
            tray::CMD_AUTOSTART => {
                self.cfg.autostart = !self.cfg.autostart;
                set_autostart(self.cfg.autostart);
                self.cfg.save();
            }
            tray::CMD_INTERVAL_250 => self.set_interval(250),
            tray::CMD_INTERVAL_500 => self.set_interval(500),
            tray::CMD_INTERVAL_1000 => self.set_interval(1000),
            tray::CMD_INTERVAL_2000 => self.set_interval(2000),
            tray::CMD_INTERVAL_5000 => self.set_interval(5000),
            tray::CMD_UNITS_BITS => self.set_units(Units::Bits),
            tray::CMD_UNITS_SI => self.set_units(Units::BytesSi),
            tray::CMD_UNITS_BIN => self.set_units(Units::BytesBin),
            tray::CMD_DEC_0 => self.set_decimals(0),
            tray::CMD_DEC_1 => self.set_decimals(1),
            tray::CMD_DEC_2 => self.set_decimals(2),
            tray::CMD_ADAPTER_AUTO => {
                self.cfg.adapter_luid = None;
                self.cfg.adapter_name.clear();
                self.cfg.save();
                self.reset_sampling();
            }
            id if id >= tray::CMD_ADAPTER_BASE => {
                let i = (id - tray::CMD_ADAPTER_BASE) as usize;
                if let Some(r) = self.adapters.get(i) {
                    self.cfg.adapter_luid = Some(r.luid);
                    self.cfg.adapter_name = if r.alias.is_empty() {
                        r.description.clone()
                    } else {
                        r.alias.clone()
                    };
                    self.cfg.save();
                    self.reset_sampling();
                }
            }
            _ => {}
        }
        true
    }

    fn set_interval(&mut self, ms: u32) {
        self.cfg.interval_ms = ms;
        self.cfg.save();
        self.reset_timer();
        self.reset_sampling();
    }

    fn set_units(&mut self, u: Units) {
        self.cfg.units = u;
        self.cfg.save();
    }

    fn set_decimals(&mut self, d: u32) {
        self.cfg.decimals = d.min(3);
        self.cfg.save();
    }

    pub fn on_taskbar_created(&mut self) {
        self.tray.re_add();
        self.taskbar_hwnd = HWND::default();
        self.overlay = None;
        self.overlay_retry_at = Instant::now();
        self.tick();
    }

    pub fn on_display_change(&mut self) {
        self.theme_at = Instant::now() - Duration::from_secs(60);
        self.tick();
    }

    pub fn on_resume(&mut self) {
        self.reset_sampling();
    }

    pub fn shutdown(&mut self) {
        self.tray.shutdown();
        self.overlay = None;
    }
}

fn set_autostart(enable: bool) {
    unsafe {
        let sub = crate::util::wide(RUN_KEY);
        let mut hk = HKEY::default();
        let rc = RegOpenKeyExW(
            HKEY_CURRENT_USER,
            PCWSTR(sub.as_ptr()),
            Some(0),
            KEY_SET_VALUE,
            &mut hk,
        );
        if rc != ERROR_SUCCESS {
            crate::util::log_error("autostart: cannot open Run key for writing");
            return;
        }
        let name = crate::util::wide(RUN_VALUE);
        if enable {
            let exe = std::env::current_exe()
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_default();
            let val = crate::util::wide(&format!("\"{exe}\""));
            let mut bytes = Vec::with_capacity(val.len() * 2);
            for u in &val {
                bytes.extend_from_slice(&u.to_le_bytes());
            }
            let _ = RegSetValueExW(hk, PCWSTR(name.as_ptr()), None, REG_SZ, Some(&bytes));
        } else {
            let _ = RegDeleteValueW(hk, PCWSTR(name.as_ptr()));
        }
        let _ = RegCloseKey(hk);
    }
}
