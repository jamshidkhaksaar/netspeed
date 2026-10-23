use crate::anchor::{self, TaskbarInfo};
use crate::config::{Config, Units};
use crate::format;
use crate::log::Logger;
use crate::net::{self, AdapterRow};
use crate::overlay::Overlay;
use crate::sampling::Sampler;
use crate::tray::{self, Tray};
use crate::{startup, theme};
use std::time::{Duration, Instant};
use windows::Win32::Foundation::HWND;
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::WindowsAndMessaging::{
    KillTimer, MB_ICONERROR, MB_OK, MessageBoxW, PostQuitMessage, SetTimer,
};
use windows::core::{PCWSTR, w};

pub struct App {
    pub cfg: Config,
    pub tray: Tray,
    overlay: Option<Overlay>,
    overlay_retry_at: Instant,
    taskbar_hwnd: HWND,
    paused: bool,
    adapters: Vec<AdapterRow>,
    selected_name: String,
    sampling: Sampler,
    sample_valid: bool,
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
            sampling: Sampler::default(),
            sample_valid: false,
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
        self.refresh_autostart();
        self.tray.add_icon();
        self.tick();
    }

    fn refresh_autostart(&mut self) {
        match startup::is_enabled() {
            Ok(enabled) => self.cfg.autostart = enabled,
            Err(e) => crate::util::log_error(&format!("cannot read Windows startup state: {e}")),
        }
    }

    pub fn start_timer(&self) {
        unsafe {
            if SetTimer(
                Some(self.tray.hwnd),
                crate::TIMER_ID,
                self.cfg.interval_clamped(),
                None,
            ) == 0
            {
                crate::util::log_error("SetTimer failed; widget cannot update");
            }
        }
    }

    fn reset_timer(&self) {
        unsafe {
            let _ = KillTimer(Some(self.tray.hwnd), crate::TIMER_ID);
        }
        self.start_timer();
    }

    pub fn tray_hwnd(&self) -> HWND {
        self.tray.hwnd
    }

    pub fn taskbar_created_msg(&self) -> u32 {
        self.taskbar_created_msg
    }

    pub fn tick(&mut self) {
        // Retry if Explorer wasn't ready during logon or temporarily lost the icon.
        self.tray.ensure_added();
        let Some(tb) = anchor::find_taskbar() else {
            self.reset_sampling();
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
                let saved = self
                    .overlay
                    .as_ref()
                    .is_some_and(|o| o.snap_bmp(self.snap_path.as_deref().unwrap_or_default()));
                if !saved {
                    crate::util::log_error("widget snapshot failed");
                }
                // Let the normal message-loop teardown remove the tray icon and
                // release GDI resources, instead of leaving a ghost tray icon.
                unsafe {
                    PostQuitMessage(if saved { 0 } else { 1 });
                }
                self.snap_path = None;
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
        self.adapters = net::list_adapters();
        self.sample_valid = false;
        if self.paused {
            return;
        }
        let Some(row) = net::pick_adapter(
            &self.adapters,
            self.cfg.adapter_luid,
            net::best_route_index(),
        ) else {
            self.selected_name = String::from("No active adapter");
            self.sampling.reset();
            return;
        };
        self.selected_name = if row.alias.is_empty() {
            row.description.clone()
        } else {
            row.alias.clone()
        };
        self.sample_valid = self.sampling.update(
            row.luid,
            row.in_octets,
            row.out_octets,
            Instant::now(),
            self.cfg.interval_clamped(),
        );
    }

    fn render(&mut self) {
        if let Some(o) = &mut self.overlay {
            let d = format::format_rate(self.sampling.down, self.cfg.units, self.cfg.decimals);
            let u = format::format_rate(self.sampling.up, self.cfg.units, self.cfg.decimals);
            if self.last_text.as_ref() == Some(&(d.clone(), u.clone())) {
                return;
            }
            o.render(&d, &u);
            self.last_text = Some((d, u));
        }
    }

    fn update_tip(&mut self) {
        let tip = if self.paused {
            format!("NetSpeed - paused\n{}", self.selected_name)
        } else {
            format!(
                "NetSpeed - {}\nD: {}  U: {}\nSession D: {}  U: {}",
                self.selected_name,
                format::format_rate(self.sampling.down, self.cfg.units, self.cfg.decimals),
                format::format_rate(self.sampling.up, self.cfg.units, self.cfg.decimals),
                format::format_bytes(self.sampling.total_down),
                format::format_bytes(self.sampling.total_up)
            )
        };
        self.tray.update_tip(&tip);
    }

    fn logging_tick(&mut self) {
        if !self.cfg.logging {
            self.logger = None;
            return;
        }
        if self.paused || !self.sample_valid {
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
            lg.log_sample(&self.selected_name, self.sampling.down, self.sampling.up);
        }
    }

    fn reset_sampling(&mut self) {
        self.sampling.reset();
        self.sample_valid = false;
    }

    pub fn toggle_show(&mut self) {
        self.cfg.show = !self.cfg.show;
        self.cfg.save();
        if let Some(o) = &mut self.overlay {
            o.set_visible(self.cfg.show);
        }
    }

    pub fn show_menu(&mut self) -> u32 {
        self.refresh_autostart();
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
                let path = crate::util::data_dir().join("logs");
                if let Err(e) = std::fs::create_dir_all(&path) {
                    crate::util::log_error(&format!("cannot create logs folder: {e}"));
                } else {
                    crate::util::open_path(&path);
                }
            }
            tray::CMD_LOGGING => {
                self.cfg.logging = !self.cfg.logging;
                self.cfg.save();
            }
            tray::CMD_AUTOSTART => {
                let enabled = !self.cfg.autostart;
                match startup::set_enabled(enabled) {
                    Ok(()) => {
                        self.cfg.autostart = enabled;
                        self.cfg.save();
                    }
                    Err(e) => {
                        let message = format!(
                            "Could not change Start with Windows: {e}\n\nCheck your Windows startup settings and try again."
                        );
                        crate::util::log_error(&message);
                        let text = crate::util::wide(&message);
                        unsafe {
                            MessageBoxW(
                                Some(self.tray.hwnd),
                                PCWSTR(text.as_ptr()),
                                w!("NetSpeed"),
                                MB_OK | MB_ICONERROR,
                            );
                        }
                    }
                }
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
        // Only the schedule changes. Keep the counters and timestamp so the
        // first tick at the new interval still counts all intervening traffic.
        self.reset_timer();
    }

    fn set_units(&mut self, u: Units) {
        self.cfg.units = u;
        self.cfg.save();
    }

    fn set_decimals(&mut self, d: u32) {
        self.cfg.decimals = d.min(2);
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
        unsafe {
            let _ = KillTimer(Some(self.tray.hwnd), crate::TIMER_ID);
        }
        self.tray.shutdown();
        self.overlay = None;
        self.logger = None;
    }
}
