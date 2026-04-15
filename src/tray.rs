use std::{
    mem::zeroed,
    sync::mpsc::{self, Receiver, Sender},
};

use tray_icon::{
    Icon, TrayIcon, TrayIconBuilder,
    menu::{Menu, MenuEvent, MenuId, MenuItem, PredefinedMenuItem},
};
use windows::Win32::{
    Foundation::{LPARAM, WPARAM},
    System::Threading::GetCurrentThreadId,
    UI::WindowsAndMessaging::{
        DispatchMessageW, GetMessageW, MSG, PostQuitMessage, PostThreadMessageW, TranslateMessage,
        WM_APP, WM_HOTKEY,
    },
};

use crate::{AppResult, hotkey};

const TRAY_EVENT_MESSAGE: u32 = WM_APP + 1;
const EXTERNAL_ACTION_MESSAGE: u32 = WM_APP + 2;

pub(crate) struct TrayApp {
    _tray_icon: TrayIcon,
    _menu: Menu,
    menu_ids: TrayMenuIds,
    menu_events: Receiver<MenuEvent>,
    external_actions: Receiver<TrayAction>,
    action_sender: TrayActionSender,
}

impl TrayApp {
    pub(crate) fn new() -> AppResult<Self> {
        let thread_id = unsafe { GetCurrentThreadId() };
        let (event_sender, menu_events) = mpsc::channel();
        let (action_sender, external_actions) = mpsc::channel();
        MenuEvent::set_event_handler(Some(move |event| {
            let _ = event_sender.send(event);
            unsafe {
                let _ = PostThreadMessageW(thread_id, TRAY_EVENT_MESSAGE, WPARAM(0), LPARAM(0));
            }
        }));

        let save_replay = MenuItem::with_id("save-replay", "Save replay", true, None);
        let pause_resume = MenuItem::with_id("pause-resume", "Pause / resume", true, None);
        let open_clips_folder =
            MenuItem::with_id("open-clips-folder", "Open clips folder", true, None);
        let open_settings = MenuItem::with_id("open-settings", "Open settings", true, None);
        let reload_settings = MenuItem::with_id("reload-settings", "Reload settings", true, None);
        let toggle_startup =
            MenuItem::with_id("toggle-startup", "Toggle start with Windows", true, None);
        let quit = MenuItem::with_id("quit", "Quit", true, None);
        let first_separator = PredefinedMenuItem::separator();
        let second_separator = PredefinedMenuItem::separator();
        let menu = Menu::new();
        menu.append_items(&[
            &save_replay,
            &pause_resume,
            &open_clips_folder,
            &first_separator,
            &open_settings,
            &reload_settings,
            &toggle_startup,
            &second_separator,
            &quit,
        ])?;

        let menu_ids = TrayMenuIds {
            save_replay: save_replay.id().clone(),
            pause_resume: pause_resume.id().clone(),
            open_clips_folder: open_clips_folder.id().clone(),
            open_settings: open_settings.id().clone(),
            reload_settings: reload_settings.id().clone(),
            toggle_startup: toggle_startup.id().clone(),
            quit: quit.id().clone(),
        };

        let tray_icon = TrayIconBuilder::new()
            .with_tooltip("Windows Rust Record")
            .with_icon(app_icon()?)
            .with_menu(Box::new(menu.clone()))
            .with_menu_on_left_click(false)
            .build()?;

        Ok(Self {
            _tray_icon: tray_icon,
            _menu: menu,
            menu_ids,
            menu_events,
            external_actions,
            action_sender: TrayActionSender {
                sender: action_sender,
                thread_id,
            },
        })
    }

    pub(crate) fn action_sender(&self) -> TrayActionSender {
        self.action_sender.clone()
    }

    pub(crate) fn run_event_loop(mut self, mut on_action: impl FnMut(TrayAction)) {
        let mut message = unsafe { zeroed::<MSG>() };
        while unsafe { GetMessageW(&mut message, None, 0, 0) }.0 > 0 {
            if message.message == WM_HOTKEY && hotkey::is_hotkey_message(&message) {
                on_action(TrayAction::SaveReplay);
                continue;
            }

            if message.message == EXTERNAL_ACTION_MESSAGE {
                self.drain_external_actions(&mut on_action);
                continue;
            }

            unsafe {
                let _ = TranslateMessage(&message);
                DispatchMessageW(&message);
            }

            self.drain_menu_events(&mut on_action);
            self.drain_external_actions(&mut on_action);
        }
    }

    fn drain_menu_events(&mut self, on_action: &mut impl FnMut(TrayAction)) {
        while let Ok(event) = self.menu_events.try_recv() {
            match self.menu_ids.action_for(event.id()) {
                Some(TrayAction::Quit) => {
                    on_action(TrayAction::Quit);
                    unsafe {
                        PostQuitMessage(0);
                    }
                }
                Some(action) => on_action(action),
                None => {}
            }
        }
    }

    fn drain_external_actions(&mut self, on_action: &mut impl FnMut(TrayAction)) {
        while let Ok(action) = self.external_actions.try_recv() {
            on_action(action);
        }
    }
}

#[derive(Clone, Copy)]
pub(crate) enum TrayAction {
    SaveReplay,
    TogglePause,
    OpenClipsFolder,
    OpenSettings,
    ReloadSettings,
    ToggleStartup,
    Quit,
}

#[derive(Clone)]
pub(crate) struct TrayActionSender {
    sender: Sender<TrayAction>,
    thread_id: u32,
}

impl TrayActionSender {
    pub(crate) fn send(&self, action: TrayAction) -> AppResult<()> {
        self.sender
            .send(action)
            .map_err(|error| format!("Could not queue tray action: {error}"))?;
        unsafe {
            PostThreadMessageW(
                self.thread_id,
                EXTERNAL_ACTION_MESSAGE,
                WPARAM(0),
                LPARAM(0),
            )?;
        }
        Ok(())
    }
}

struct TrayMenuIds {
    save_replay: MenuId,
    pause_resume: MenuId,
    open_clips_folder: MenuId,
    open_settings: MenuId,
    reload_settings: MenuId,
    toggle_startup: MenuId,
    quit: MenuId,
}

impl TrayMenuIds {
    fn action_for(&self, id: &MenuId) -> Option<TrayAction> {
        if id == &self.save_replay {
            Some(TrayAction::SaveReplay)
        } else if id == &self.pause_resume {
            Some(TrayAction::TogglePause)
        } else if id == &self.open_clips_folder {
            Some(TrayAction::OpenClipsFolder)
        } else if id == &self.open_settings {
            Some(TrayAction::OpenSettings)
        } else if id == &self.reload_settings {
            Some(TrayAction::ReloadSettings)
        } else if id == &self.toggle_startup {
            Some(TrayAction::ToggleStartup)
        } else if id == &self.quit {
            Some(TrayAction::Quit)
        } else {
            None
        }
    }
}

fn app_icon() -> AppResult<Icon> {
    const SIZE: u32 = 32;
    let mut rgba = vec![0_u8; (SIZE * SIZE * 4) as usize];

    let badge = [255, 255, 255, 255];
    let badge_edge = [214, 226, 239, 255];
    let camera_shadow = [0, 0, 0, 90];
    let camera_body = [31, 37, 49, 255];
    let camera_top = [45, 55, 73, 255];
    let camera_edge = [232, 240, 248, 255];
    let lens_outer = [231, 246, 255, 255];
    let lens_inner = [24, 119, 242, 255];
    let lens_core = [6, 45, 98, 255];
    let highlight = [255, 210, 71, 255];
    let highlight_edge = [255, 255, 255, 255];

    draw_circle(&mut rgba, SIZE, 16, 16, 15, badge_edge);
    draw_circle(&mut rgba, SIZE, 16, 16, 14, badge);
    draw_rounded_rect(&mut rgba, SIZE, 4, 12, 25, 15, 4, camera_shadow);
    draw_rounded_rect(&mut rgba, SIZE, 3, 10, 26, 16, 4, camera_edge);
    draw_rounded_rect(&mut rgba, SIZE, 5, 12, 22, 12, 3, camera_body);
    draw_rounded_rect(&mut rgba, SIZE, 9, 7, 10, 6, 2, camera_top);
    draw_rect(&mut rgba, SIZE, 21, 13, 4, 3, [87, 102, 130, 255]);

    draw_circle(&mut rgba, SIZE, 16, 18, 7, lens_outer);
    draw_circle(&mut rgba, SIZE, 16, 18, 5, lens_inner);
    draw_circle(&mut rgba, SIZE, 16, 18, 3, lens_core);
    draw_circle(&mut rgba, SIZE, 14, 16, 1, [255, 255, 255, 230]);

    draw_spark(&mut rgba, SIZE, 24, 7, highlight_edge);
    draw_spark(&mut rgba, SIZE, 24, 7, highlight);

    Icon::from_rgba(rgba, SIZE, SIZE).map_err(Into::into)
}

fn draw_rect(rgba: &mut [u8], size: u32, x: i32, y: i32, width: i32, height: i32, color: [u8; 4]) {
    for row in y..y + height {
        for col in x..x + width {
            set_pixel(rgba, size, col, row, color);
        }
    }
}

fn draw_rounded_rect(
    rgba: &mut [u8],
    size: u32,
    x: i32,
    y: i32,
    width: i32,
    height: i32,
    radius: i32,
    color: [u8; 4],
) {
    for row in y..y + height {
        for col in x..x + width {
            let dx = if col < x + radius {
                x + radius - col
            } else if col >= x + width - radius {
                col - (x + width - radius - 1)
            } else {
                0
            };
            let dy = if row < y + radius {
                y + radius - row
            } else if row >= y + height - radius {
                row - (y + height - radius - 1)
            } else {
                0
            };

            if dx == 0 || dy == 0 || dx * dx + dy * dy <= radius * radius {
                set_pixel(rgba, size, col, row, color);
            }
        }
    }
}

fn draw_circle(
    rgba: &mut [u8],
    size: u32,
    center_x: i32,
    center_y: i32,
    radius: i32,
    color: [u8; 4],
) {
    let radius_sq = radius * radius;
    for row in center_y - radius..=center_y + radius {
        for col in center_x - radius..=center_x + radius {
            let dx = col - center_x;
            let dy = row - center_y;
            if dx * dx + dy * dy <= radius_sq {
                set_pixel(rgba, size, col, row, color);
            }
        }
    }
}

fn draw_spark(rgba: &mut [u8], size: u32, center_x: i32, center_y: i32, color: [u8; 4]) {
    for offset in -4..=4 {
        set_pixel(rgba, size, center_x + offset, center_y, color);
        set_pixel(rgba, size, center_x, center_y + offset, color);
    }
    for offset in -2..=2 {
        set_pixel(rgba, size, center_x + offset, center_y + offset, color);
        set_pixel(rgba, size, center_x + offset, center_y - offset, color);
    }
}

fn set_pixel(rgba: &mut [u8], size: u32, x: i32, y: i32, color: [u8; 4]) {
    if x < 0 || y < 0 || x >= size as i32 || y >= size as i32 {
        return;
    }

    let index = ((y as u32 * size + x as u32) * 4) as usize;
    rgba[index..index + 4].copy_from_slice(&color);
}
