mod mpris;
mod ui;

use gtk::{gio, glib, prelude::*};
use may_music::{
    engine::{self, Command, UiEvent},
    store::data_dir,
};
use std::{cell::RefCell, rc::Rc};

fn main() -> glib::ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "--help" || a == "-h") {
        println!(
            "Mây Music 0.1.0\n\nmay-music [--background | --show | --hide | --play | --pause | --toggle | --next | --previous | --stop | --quit | --diagnose]\n\nKhông có tham số: mở cửa sổ hoặc hiện lại app đang chạy.\n--play: phát / tiếp tục; đang phát thì giữ nguyên, sau stop thì phát lại từ đầu.\n--pause: tạm dừng, giữ vị trí đang nghe.\n--toggle: đổi giữa phát và tạm dừng.\n--stop: dừng phát và đưa vị trí về đầu bài, vẫn giữ app chạy.\n--background: khởi động ẩn, không tự phát nhạc.\n--show / --hide: hiện / ẩn cửa sổ.\n--next / --previous: bài tiếp theo / bài trước.\n--quit: thoát hoàn toàn.\n--diagnose: kiểm tra dependency và đường dẫn dữ liệu."
        );
        return glib::ExitCode::SUCCESS;
    }
    if args.iter().any(|a| a == "--diagnose") {
        for (label, binary, arg) in [
            (
                "mpv",
                std::env::var("MAY_MPV").unwrap_or_else(|_| "mpv".into()),
                "--version",
            ),
            (
                "yt-dlp",
                std::env::var("MAY_YTDLP").unwrap_or_else(|_| "yt-dlp".into()),
                "--version",
            ),
            ("node", "node".into(), "--version"),
        ] {
            match std::process::Command::new(binary).arg(arg).output() {
                Ok(output) if output.status.success() => println!(
                    "{label}: {}",
                    String::from_utf8_lossy(&output.stdout)
                        .lines()
                        .next()
                        .unwrap_or("OK")
                ),
                _ => println!("{label}: CHƯA CÓ / không chạy được"),
            }
        }
        println!("Dữ liệu: {}", data_dir().display());
        return glib::ExitCode::SUCCESS;
    }
    let valid = [
        "--background",
        "--show",
        "--hide",
        "--play",
        "--pause",
        "--toggle",
        "--next",
        "--previous",
        "--stop",
        "--quit",
    ];
    if args.len() > 1 || args.first().is_some_and(|a| !valid.contains(&a.as_str())) {
        eprintln!("Tham số không hợp lệ. Dùng --help để xem hướng dẫn.");
        return glib::ExitCode::FAILURE;
    }
    let app = gtk::Application::builder()
        .application_id("io.github.maymusic.Player")
        .flags(gio::ApplicationFlags::HANDLES_COMMAND_LINE)
        .build();
    for option in valid {
        app.add_main_option(
            &option[2..],
            glib::Char::from(0),
            glib::OptionFlags::NONE,
            glib::OptionArg::None,
            option,
            None,
        );
    }
    let worker = Rc::new(RefCell::new(None));
    let target = worker.clone();
    app.connect_startup(move |app| match setup(app) {
        Ok(worker) => *target.borrow_mut() = Some(worker),
        Err(error) => {
            eprintln!("Không mở được thư viện nhạc: {error:#}");
            app.quit();
        }
    });
    app.connect_command_line(|app, command_line| {
        let options = command_line.options_dict();
        let action = [
            "play", "pause", "toggle", "next", "previous", "stop", "hide", "quit",
        ]
        .into_iter()
        .find(|name| options.contains(name));
        if let Some(action) = action {
            if command_line.is_remote() {
                app.activate_action(action, None);
            } else if action == "play" {
                app.activate();
                app.activate_action(action, None);
            } else {
                eprintln!("Mây Music chưa chạy. Mở app trước.");
                app.quit();
            }
        } else if !options.contains("background") {
            app.activate();
        }
        glib::ExitCode::SUCCESS
    });
    let all_args: Vec<String> = std::env::args().collect();
    let result = app.run_with_args(&all_args);
    drop(worker.borrow_mut().take());
    result
}

fn setup(app: &gtk::Application) -> anyhow::Result<engine::Worker> {
    let (handle, events, worker) = engine::launch()?;
    let state = Rc::new(RefCell::new(None));
    let ui = ui::Ui::new(app, handle.clone(), state.clone());
    for (name, command) in [
        ("play", Command::Pause(false)),
        ("pause", Command::Pause(true)),
        ("toggle", Command::Toggle),
        ("next", Command::Next),
        ("previous", Command::Previous),
        ("stop", Command::Stop),
    ] {
        let action = gio::SimpleAction::new(name, None);
        let h = handle.clone();
        action.connect_activate(move |_, _| h.send(command.clone()));
        app.add_action(&action);
    }
    let hide = gio::SimpleAction::new("hide", None);
    let window = ui.window.downgrade();
    hide.connect_activate(move |_, _| {
        if let Some(window) = window.upgrade() {
            window.set_visible(false);
        }
    });
    app.add_action(&hide);
    let quit = gio::SimpleAction::new("quit", None);
    let weak = app.downgrade();
    quit.connect_activate(move |_, _| {
        if let Some(app) = weak.upgrade() {
            app.quit();
        }
    });
    app.add_action(&quit);
    app.set_accels_for_action("app.toggle", &["<Control>space"]);
    app.set_accels_for_action("app.next", &["<Control>Right"]);
    app.set_accels_for_action("app.previous", &["<Control>Left"]);
    app.set_accels_for_action("app.hide", &["<Control>w"]);
    app.set_accels_for_action("app.quit", &["<Control>q"]);
    let window = ui.window.downgrade();
    app.connect_activate(move |_| {
        if let Some(window) = window.upgrade() {
            window.present();
        }
    });
    let mpris = match mpris::Mpris::new(app, state, &handle) {
        Ok(mpris) => Some(Rc::new(mpris)),
        Err(e) => {
            ui.message(&format!("Chưa bật được điều khiển media của Linux: {e}"));
            None
        }
    };
    let target_ui = ui.clone();
    glib::spawn_future_local(async move {
        while let Ok(event) = events.recv().await {
            match event {
                UiEvent::State(snapshot) => {
                    if let Some(mpris) = &mpris {
                        mpris.changed(&snapshot);
                    }
                    target_ui.render(*snapshot);
                }
                UiEvent::Progress(position, duration) => target_ui.progress(position, duration),
                UiEvent::SearchStarted => target_ui.searching(),
                UiEvent::SearchResults(tracks) => target_ui.results(tracks),
                UiEvent::SearchError(error) => target_ui.search_error(&error),
                UiEvent::Message(message) => target_ui.message(&message),
            }
        }
    });
    Ok(worker)
}
