// release — без консоли; debug — с консолью для логов
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use anyhow::Result;

mod autostart;
mod config;
mod desktop;
mod hotkey;
mod overlay;
mod slide_overlay;
mod touch;
mod tray;
mod wheel;

use config::Config;
use desktop::DesktopManager;
use hotkey::Action;
use windows::Win32::UI::WindowsAndMessaging::{DispatchMessageW, GetMessageW, MSG, TranslateMessage};

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if let Some(cmd) = args.first() {
        match cmd.as_str() {
            "--enable-autostart" => {
                autostart::enable()?;
                println!("autostart enabled");
                return Ok(());
            }
            "--disable-autostart" => {
                autostart::disable()?;
                println!("autostart disabled");
                return Ok(());
            }
            "--help" | "-h" => {
                println!(
                    "Usage: win_desktop_swither [--enable-autostart | --disable-autostart]"
                );
                return Ok(());
            }
            _ => {}
        }
    }

    println!("Win Desktop Switcher\n");

    let (cfg, path) = Config::load()?;
    match &path {
        Some(p) => println!("[config] loaded from: {}", p.display()),
        None => println!("[config] no config.toml found, using defaults"),
    }
    println!(
        "  display.mode = {:?}, anim = {}/{}ms/{}, wheel = {}/{}ms/{}, wrap = {}",
        cfg.display.mode,
        cfg.animation.mode,
        cfg.animation.duration_ms,
        cfg.animation.easing,
        cfg.wheel.threshold,
        cfg.wheel.cooldown_ms,
        cfg.wheel.velocity_damping,
        cfg.navigation.wrap,
    );
    println!();

    // DesktopManager привязан к COM-apartment — держим его в отдельном worker'е
    let (tx, rx) = crossbeam_channel::unbounded::<Action>();
    let cfg_worker = cfg.clone();
    let _worker = std::thread::Builder::new()
        .name("desktop-worker".into())
        .spawn(move || {
            let mgr = match DesktopManager::new() {
                Ok(m) => m,
                Err(e) => {
                    eprintln!("[worker] failed to init winvd: {e:?}");
                    return;
                }
            };
            match mgr.current_index() {
                Ok(idx) => println!("[worker] ready, current desktop: {idx}"),
                Err(e) => eprintln!("[worker] init warn: {e:?}"),
            }
            for action in rx {
                if let Err(e) = handle_action(action, &mgr, &cfg_worker) {
                    eprintln!("[action error] {action:?}: {e:?}");
                }
            }
        })?;

    hotkey::install(tx, cfg.wheel.cooldown_ms)?;
    println!("[hook] keyboard hook installed (cooldown {} ms)", cfg.wheel.cooldown_ms);

    wheel::install(
        cfg.wheel.threshold,
        cfg.wheel.cooldown_ms,
        cfg.wheel.velocity_damping,
        cfg.wheel.hot_zone_height_px,
    )?;
    println!(
        "[hook] mouse wheel hook installed (threshold {}, damping {}, hot_zone {} px)",
        cfg.wheel.threshold, cfg.wheel.velocity_damping, cfg.wheel.hot_zone_height_px,
    );

    match touch::init(&cfg.touch) {
        Ok(()) if cfg.touch.enabled => println!(
            "[touch] injector initialized: distance {} px, {} steps, {} ms/step, easing {}",
            cfg.touch.swipe_distance_px,
            cfg.touch.swipe_steps,
            cfg.touch.step_delay_ms,
            cfg.touch.easing,
        ),
        Ok(()) => println!("[touch] disabled via config — using winvd fallback"),
        Err(e) => eprintln!("[touch] init failed: {e:?} — fallback to winvd"),
    }

    overlay::install()?;
    tray::install()?;
    println!(
        "[tray] icon installed (autostart = {})",
        autostart::is_enabled()
    );

    println!("[ready] Win+←/→, Win+wheel, Win+1..9\n");

    run_message_loop();

    Ok(())
}

fn handle_action(action: Action, mgr: &DesktopManager, cfg: &Config) -> Result<()> {
    let count = mgr.count()?;
    let (did_switch, direction) = match action {
        Action::SwitchPrev => {
            let (ok, dir, _) = switch_relative(mgr, cfg, -1)?;
            (ok, dir)
        }
        Action::SwitchNext => {
            let (ok, dir, _) = switch_relative(mgr, cfg, 1)?;
            (ok, dir)
        }
        Action::SwitchTo(idx) => {
            if idx < count {
                mgr.switch_to(idx)?;
                (true, 0_i32)
            } else {
                (false, 0_i32)
            }
        }
        Action::MovePrev | Action::MoveNext => {
            eprintln!("{action:?} not implemented");
            (false, 0_i32)
        }
    };

    if did_switch && cfg.overlay.enabled {
        let new_idx = mgr.current_index()?;
        overlay::show(new_idx, count, 1100, direction);
    }
    Ok(())
}

fn switch_relative(
    mgr: &DesktopManager,
    cfg: &Config,
    direction: i32,
) -> Result<(bool, i32, bool)> {
    match cfg.animation.mode.as_str() {
        "slide-push" => {
            let wrap = cfg.navigation.wrap;
            let r = slide_overlay::play_slide_push(
                direction,
                cfg.animation.duration_ms,
                &cfg.animation.easing,
                || mgr.switch_relative(direction, wrap),
            );
            if let Err(e) = r {
                eprintln!("[slide-push] failed: {e:?} — fallback instant");
                mgr.switch_relative(direction, wrap)?;
                return Ok((true, direction, true));
            }
            Ok((true, direction, false))
        }
        "instant" => {
            mgr.switch_relative(direction, cfg.navigation.wrap)?;
            Ok((true, direction, true))
        }
        _ => {
            // touch и любое неизвестное — системный swipe injection
            match touch::swipe(direction) {
                Ok(()) => Ok((true, direction, false)),
                Err(_) => {
                    mgr.switch_relative(direction, cfg.navigation.wrap)?;
                    Ok((true, direction, true))
                }
            }
        }
    }
}

fn run_message_loop() {
    let mut msg = MSG::default();
    unsafe {
        while GetMessageW(&mut msg, None, 0, 0).into() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}
