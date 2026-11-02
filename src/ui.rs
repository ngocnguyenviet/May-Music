use gtk::{gio, glib, prelude::*};
use may_music::{
    engine::{Command, Handle, Phase, Snapshot},
    model::{Track, clock},
};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

pub struct Ui {
    pub window: gtk::ApplicationWindow,
    pub state: Rc<RefCell<Option<Snapshot>>>,
    handle: Handle,
    playlists: gtk::ListBox,
    results: gtk::ListBox,
    tracks: gtk::ListBox,
    stack: gtk::Stack,
    search_info: gtk::Label,
    search_spinner: gtk::Spinner,
    library_title: gtk::Label,
    library_info: gtk::Label,
    library_play: gtk::Button,
    current_title: gtk::Label,
    current_artist: gtk::Label,
    playback_status: gtk::Label,
    play: gtk::Button,
    autoplay: gtk::Switch,
    seek: gtk::Scale,
    elapsed: gtk::Label,
    duration: gtk::Label,
    volume: gtk::Scale,
    message: gtk::Label,
    updating: Cell<bool>,
    rendered_revision: Cell<Option<u64>>,
}

fn text(value: &str, class: &str) -> gtk::Label {
    let label = gtk::Label::new(Some(value));
    label.set_xalign(0.0);
    if !class.is_empty() {
        label.add_css_class(class);
    }
    label
}
fn icon_button(icon: &str, tip: &str) -> gtk::Button {
    let button = gtk::Button::from_icon_name(icon);
    button.set_tooltip_text(Some(tip));
    button.set_accessible_role(gtk::AccessibleRole::Button);
    button
}
fn margins(widget: &impl IsA<gtk::Widget>, size: i32) {
    widget.set_margin_start(size);
    widget.set_margin_end(size);
    widget.set_margin_top(size);
    widget.set_margin_bottom(size);
}
fn clear(list: &gtk::ListBox) {
    while let Some(child) = list.first_child() {
        list.remove(&child);
    }
}
fn scroller(child: &impl IsA<gtk::Widget>) -> gtk::ScrolledWindow {
    gtk::ScrolledWindow::builder()
        .child(child)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vexpand(true)
        .build()
}
fn entry_popover(button: &gtk::MenuButton, placeholder: &str, submit: impl Fn(String) + 'static) {
    let pop = gtk::Popover::new();
    let content = gtk::Box::new(gtk::Orientation::Vertical, 10);
    let entry = gtk::Entry::builder()
        .placeholder_text(placeholder)
        .width_chars(24)
        .max_length(80)
        .build();
    let save = gtk::Button::with_label("Lưu playlist");
    save.add_css_class("accent");
    content.append(&entry);
    content.append(&save);
    pop.set_child(Some(&content));
    button.set_popover(Some(&pop));
    let entry2 = entry.clone();
    let pop2 = pop.clone();
    let submit = Rc::new(submit);
    let submit2 = submit.clone();
    save.connect_clicked(move |_| {
        if !entry2.text().trim().is_empty() {
            submit(entry2.text().to_string());
            entry2.set_text("");
            pop2.popdown();
        }
    });
    entry.connect_activate(move |entry| {
        if !entry.text().trim().is_empty() {
            submit2(entry.text().to_string());
            entry.set_text("");
            pop.popdown();
        }
    });
}

impl Ui {
    pub fn new(
        app: &gtk::Application,
        handle: Handle,
        state: Rc<RefCell<Option<Snapshot>>>,
    ) -> Rc<Self> {
        let css = gtk::CssProvider::new();
        css.load_from_data(include_str!("style.css"));
        gtk::style_context_add_provider_for_display(
            &gtk::gdk::Display::default().unwrap(),
            &css,
            gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );
        let window = gtk::ApplicationWindow::builder()
            .application(app)
            .title("Mây Music")
            .default_width(1100)
            .default_height(760)
            .build();
        window.set_size_request(860, 620);
        window.set_hide_on_close(true);
        window.set_icon_name(Some("io.github.maymusic.Player"));
        let header = gtk::HeaderBar::new();
        header.set_show_title_buttons(false);
        let head_title = text("Mây Music", "track-title");
        header.set_title_widget(Some(&head_title));
        let background = gtk::Button::with_label("Chạy nền");
        background.add_css_class("flat");
        background.set_tooltip_text(Some("Ẩn cửa sổ, tiếp tục nghe. Mở lại app để hiện cửa sổ."));
        let weak = window.downgrade();
        background.connect_clicked(move |_| {
            if let Some(w) = weak.upgrade() {
                w.set_visible(false);
            }
        });
        header.pack_end(&background);
        let menu = gio::Menu::new();
        menu.append(Some("Dừng phát"), Some("app.stop"));
        menu.append(Some("Thoát hoàn toàn"), Some("app.quit"));
        let menu_button = gtk::MenuButton::builder()
            .icon_name("open-menu-symbolic")
            .menu_model(&menu)
            .build();
        header.pack_end(&menu_button);
        window.set_titlebar(Some(&header));

        let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
        let body = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        body.set_vexpand(true);
        let side = gtk::Box::new(gtk::Orientation::Vertical, 14);
        side.set_width_request(216);
        side.set_hexpand(false);
        side.add_css_class("sidebar");
        margins(&side, 0);
        let side_inner = gtk::Box::new(gtk::Orientation::Vertical, 14);
        margins(&side_inner, 20);
        side_inner.set_vexpand(true);
        side_inner.append(&text("☁  mây", "brand"));
        let tagline = text("NHẠC CHO MỖI NGÀY", "eyebrow");
        tagline.set_wrap(true);
        tagline.set_max_width_chars(20);
        side_inner.append(&tagline);
        let discover = gtk::Button::with_label("⌕   Tìm nhạc YouTube");
        discover.add_css_class("accent");
        discover.set_margin_top(20);
        side_inner.append(&discover);
        let section = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        section.set_margin_top(16);
        let label = text("PLAYLIST CỦA TỚ", "eyebrow");
        label.set_hexpand(true);
        let new_playlist = gtk::MenuButton::builder()
            .icon_name("list-add-symbolic")
            .tooltip_text("Tạo playlist")
            .build();
        new_playlist.add_css_class("flat");
        let h = handle.clone();
        entry_popover(&new_playlist, "Tên playlist mới", move |name| {
            h.send(Command::Create(name))
        });
        section.append(&label);
        section.append(&new_playlist);
        side_inner.append(&section);
        let playlists = gtk::ListBox::new();
        playlists.set_selection_mode(gtk::SelectionMode::Single);
        side_inner.append(&scroller(&playlists));
        let side_note = text(
            "Chỉ âm thanh. Không video.\nĐóng cửa sổ vẫn tiếp tục nghe.",
            "muted",
        );
        side_note.add_css_class("small");
        side_note.set_wrap(true);
        side_inner.append(&side_note);
        side.append(&side_inner);
        body.append(&side);

        let main = gtk::Box::new(gtk::Orientation::Vertical, 16);
        main.set_hexpand(true);
        margins(&main, 28);
        let intro = gtk::Box::new(gtk::Orientation::Horizontal, 10);
        let intro_copy = gtk::Box::new(gtk::Orientation::Vertical, 5);
        intro_copy.set_hexpand(true);
        let hero = text("Một chút nhạc, cho ngày của cậu.", "hero");
        hero.set_wrap(true);
        hero.set_max_width_chars(38);
        intro_copy.append(&hero);
        let subtitle = text(
            "Tìm giai điệu yêu thích. Lưu lại. Và cứ để nhạc chạy.",
            "muted",
        );
        subtitle.set_wrap(true);
        intro_copy.append(&subtitle);
        intro.append(&intro_copy);
        let badge = text("YOUTUBE · AUDIO", "badge");
        badge.set_valign(gtk::Align::Start);
        intro.append(&badge);
        main.append(&intro);
        let search_bar = gtk::Box::new(gtk::Orientation::Horizontal, 10);
        let search = gtk::Entry::builder()
            .placeholder_text("Tên bài hát, nghệ sĩ hoặc link YouTube…")
            .hexpand(true)
            .max_length(2048)
            .primary_icon_name("system-search-symbolic")
            .build();
        search.set_tooltip_text(Some("Nhập từ khóa hoặc dán link YouTube rồi nhấn Enter"));
        let search_button = gtk::Button::with_label("Tìm nhạc");
        search_button.add_css_class("accent");
        search_bar.append(&search);
        search_bar.append(&search_button);
        main.append(&search_bar);

        let stack = gtk::Stack::new();
        stack.set_vexpand(true);
        stack.set_transition_type(gtk::StackTransitionType::Crossfade);
        let search_page = gtk::Box::new(gtk::Orientation::Vertical, 10);
        let result_header = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let search_info = text("KHÁM PHÁ GIAI ĐIỆU MỚI", "eyebrow");
        search_info.set_hexpand(true);
        let search_spinner = gtk::Spinner::new();
        result_header.append(&search_info);
        result_header.append(&search_spinner);
        search_page.append(&result_header);
        let results = gtk::ListBox::new();
        results.set_selection_mode(gtk::SelectionMode::None);
        results.add_css_class("tracks");
        let empty = gtk::Box::new(gtk::Orientation::Vertical, 12);
        empty.set_valign(gtk::Align::Center);
        empty.set_vexpand(true);
        margins(&empty, 30);
        let music_icon = gtk::Image::from_icon_name("audio-x-generic-symbolic");
        music_icon.set_pixel_size(56);
        music_icon.add_css_class("muted");
        empty.append(&music_icon);
        let title = text("Bật một giai điệu.", "section-title");
        title.set_halign(gtk::Align::Center);
        empty.append(&title);
        let note = text(
            "Tìm bài, nghệ sĩ hoặc dán link YouTube ở phía trên.\nNhấn + để thêm bài vào playlist đang chọn.",
            "muted",
        );
        note.set_justify(gtk::Justification::Center);
        note.set_halign(gtk::Align::Center);
        empty.append(&note);
        results.set_placeholder(Some(&empty));
        search_page.append(&scroller(&results));
        stack.add_named(&search_page, Some("search"));

        let library_page = gtk::Box::new(gtk::Orientation::Vertical, 10);
        let library_header = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let lib_copy = gtk::Box::new(gtk::Orientation::Vertical, 4);
        lib_copy.set_hexpand(true);
        let library_title = text("Nhạc của tớ", "section-title");
        library_title.set_ellipsize(gtk::pango::EllipsizeMode::End);
        lib_copy.append(&library_title);
        let library_info = text("0 bài hát · lưu trên máy", "muted");
        lib_copy.append(&library_info);
        library_header.append(&lib_copy);
        let rename = gtk::MenuButton::builder()
            .icon_name("document-edit-symbolic")
            .tooltip_text("Đổi tên playlist")
            .build();
        let h = handle.clone();
        entry_popover(&rename, "Tên mới", move |name| {
            h.send(Command::Rename(name))
        });
        library_header.append(&rename);
        let delete = icon_button("user-trash-symbolic", "Xóa playlist");
        library_header.append(&delete);
        let library_play = gtk::Button::with_label("▶  Phát playlist");
        library_play.add_css_class("accent");
        library_header.append(&library_play);
        library_page.append(&library_header);
        let tracks = gtk::ListBox::new();
        tracks.add_css_class("tracks");
        tracks.set_selection_mode(gtk::SelectionMode::None);
        let empty = text(
            "Playlist còn trống. Tìm nhạc và nhấn + để thêm bài.",
            "muted",
        );
        margins(&empty, 28);
        empty.set_wrap(true);
        tracks.set_placeholder(Some(&empty));
        library_page.append(&scroller(&tracks));
        stack.add_named(&library_page, Some("library"));
        main.append(&stack);
        let message = text("", "message");
        message.set_wrap(true);
        message.set_visible(false);
        main.append(&message);
        body.append(&main);
        root.append(&body);

        let player_panel = gtk::Box::new(gtk::Orientation::Vertical, 8);
        player_panel.add_css_class("now-playing");
        let panel = gtk::Box::new(gtk::Orientation::Vertical, 10);
        panel.set_margin_start(24);
        panel.set_margin_end(24);
        panel.set_margin_top(14);
        panel.set_margin_bottom(14);
        let player_row = gtk::Box::new(gtk::Orientation::Horizontal, 18);
        let cover = gtk::Image::from_icon_name("audio-x-generic-symbolic");
        cover.set_pixel_size(28);
        cover.add_css_class("album-tile");
        player_row.append(&cover);
        let now_copy = gtk::Box::new(gtk::Orientation::Vertical, 3);
        now_copy.set_hexpand(true);
        let current_title = text("Chưa có bài đang phát", "track-title");
        current_title.set_ellipsize(gtk::pango::EllipsizeMode::End);
        current_title.set_max_width_chars(38);
        let current_artist = text("Chọn bài từ tìm kiếm hoặc playlist", "muted");
        current_artist.set_ellipsize(gtk::pango::EllipsizeMode::End);
        let playback_status = text("Sẵn sàng", "status");
        playback_status.set_ellipsize(gtk::pango::EllipsizeMode::End);
        now_copy.append(&current_title);
        now_copy.append(&current_artist);
        now_copy.append(&playback_status);
        player_row.append(&now_copy);
        let previous = icon_button(
            "media-skip-backward-symbolic",
            "Bài trước / phát lại từ đầu",
        );
        previous.add_css_class("transport");
        let play = icon_button(
            "media-playback-start-symbolic",
            "Phát / tạm dừng (Ctrl+Space)",
        );
        play.add_css_class("play");
        let next = icon_button("media-skip-forward-symbolic", "Bài tiếp theo (Ctrl+Right)");
        next.add_css_class("transport");
        player_row.append(&previous);
        player_row.append(&play);
        player_row.append(&next);
        let auto_copy = gtk::Box::new(gtk::Orientation::Vertical, 3);
        auto_copy.set_margin_start(14);
        auto_copy.append(&text("Tự phát đề xuất", "track-title"));
        let auto_note = text("Khi hết playlist", "small");
        auto_note.add_css_class("muted");
        auto_copy.append(&auto_note);
        player_row.append(&auto_copy);
        let autoplay = gtk::Switch::new();
        autoplay.set_valign(gtk::Align::Center);
        autoplay.set_tooltip_text(Some(
            "Bật: tìm bài từ YouTube Mix / bài liên quan. Tắt: hết playlist thì lặp bài hiện tại.",
        ));
        player_row.append(&autoplay);
        panel.append(&player_row);
        let progress = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        let elapsed = text("0:00", "small");
        elapsed.add_css_class("muted");
        elapsed.set_width_chars(7);
        let duration = text("0:00", "small");
        duration.add_css_class("muted");
        duration.set_width_chars(7);
        let seek = gtk::Scale::with_range(gtk::Orientation::Horizontal, 0.0, 1.0, 1.0);
        seek.set_hexpand(true);
        seek.set_draw_value(false);
        seek.set_sensitive(false);
        progress.append(&elapsed);
        progress.append(&seek);
        progress.append(&duration);
        let volume_icon = gtk::Image::from_icon_name("audio-volume-high-symbolic");
        progress.append(&volume_icon);
        let volume = gtk::Scale::with_range(gtk::Orientation::Horizontal, 0.0, 100.0, 1.0);
        volume.set_draw_value(false);
        volume.set_width_request(100);
        volume.set_tooltip_text(Some("Âm lượng"));
        progress.append(&volume);
        panel.append(&progress);
        player_panel.append(&panel);
        root.append(&player_panel);
        window.set_child(Some(&root));

        let ui = Rc::new(Self {
            window,
            state,
            handle: handle.clone(),
            playlists,
            results,
            tracks,
            stack,
            search_info,
            search_spinner,
            library_title,
            library_info,
            library_play,
            current_title,
            current_artist,
            playback_status,
            play: play.clone(),
            autoplay: autoplay.clone(),
            seek: seek.clone(),
            elapsed,
            duration,
            volume: volume.clone(),
            message,
            updating: Cell::new(false),
            rendered_revision: Cell::new(None),
        });
        let weak = Rc::downgrade(&ui);
        discover.connect_clicked(move |_| {
            if let Some(ui) = weak.upgrade() {
                ui.stack.set_visible_child_name("search");
            }
        });
        let weak = Rc::downgrade(&ui);
        let search2 = search.clone();
        search_button.connect_clicked(move |_| {
            if let Some(ui) = weak.upgrade() {
                ui.search(&search2.text());
            }
        });
        let weak = Rc::downgrade(&ui);
        search.connect_activate(move |entry| {
            if let Some(ui) = weak.upgrade() {
                ui.search(&entry.text());
            }
        });
        let h = handle.clone();
        play.connect_clicked(move |_| h.send(Command::Toggle));
        let h = handle.clone();
        previous.connect_clicked(move |_| h.send(Command::Previous));
        let h = handle.clone();
        next.connect_clicked(move |_| h.send(Command::Next));
        let h = handle.clone();
        ui.library_play
            .connect_clicked(move |_| h.send(Command::PlayPlaylist(0)));
        let weak = Rc::downgrade(&ui);
        ui.playlists.connect_row_activated(move |_, row| {
            if let Some(ui) = weak.upgrade()
                && let Some(state) = ui.state.borrow().as_ref()
                && let Some(p) = state.library.playlists.get(row.index() as usize)
            {
                ui.handle.send(Command::Select(p.id));
                ui.stack.set_visible_child_name("library");
            }
        });
        let weak = Rc::downgrade(&ui);
        autoplay.connect_active_notify(move |switch| {
            if let Some(ui) = weak.upgrade()
                && !ui.updating.get()
            {
                ui.handle.send(Command::Autoplay(switch.is_active()));
            }
        });
        let h = handle.clone();
        seek.connect_change_value(move |_, _, value| {
            h.send(Command::Seek(value));
            glib::Propagation::Proceed
        });
        let weak = Rc::downgrade(&ui);
        volume.connect_change_value(move |_, _, value| {
            if let Some(ui) = weak.upgrade()
                && !ui.updating.get()
            {
                ui.handle.send(Command::Volume(value));
            }
            glib::Propagation::Proceed
        });
        let weak = Rc::downgrade(&ui);
        delete.connect_clicked(move |_| {
            if let Some(ui) = weak.upgrade() {
                let dialog = gtk::MessageDialog::builder()
                    .transient_for(&ui.window)
                    .modal(true)
                    .message_type(gtk::MessageType::Question)
                    .buttons(gtk::ButtonsType::OkCancel)
                    .text("Xóa playlist đang chọn?")
                    .secondary_text("Danh sách này sẽ được xóa khỏi máy.")
                    .build();
                let h = ui.handle.clone();
                dialog.connect_response(move |dialog, response| {
                    if response == gtk::ResponseType::Ok {
                        h.send(Command::DeletePlaylist);
                    }
                    dialog.close();
                });
                dialog.present();
            }
        });
        ui
    }

    fn search(&self, query: &str) {
        if !query.trim().is_empty() {
            self.stack.set_visible_child_name("search");
            self.handle.send(Command::Search(query.to_string()));
        }
    }
    pub fn searching(&self) {
        self.search_info.set_text("ĐANG TÌM TRÊN YOUTUBE…");
        self.search_spinner.start();
    }
    pub fn search_error(&self, error: &str) {
        self.search_spinner.stop();
        self.search_info
            .set_text("CHƯA TÌM ĐƯỢC · NHẤN TÌM NHẠC ĐỂ THỬ LẠI");
        self.message(error);
    }
    pub fn results(&self, tracks: Vec<Track>) {
        self.search_spinner.stop();
        self.search_info.set_text(&format!(
            "{} KẾT QUẢ · THÊM VÀO PLAYLIST ĐANG CHỌN",
            tracks.len()
        ));
        clear(&self.results);
        for (i, track) in tracks.into_iter().enumerate() {
            self.results.append(&self.track_row(&track, i, false));
        }
    }
    fn track_row(&self, track: &Track, index: usize, library: bool) -> gtk::Box {
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 10);
        let number = text(&format!("{:02}", index + 1), "track-number");
        row.append(&number);
        let copy = gtk::Box::new(gtk::Orientation::Vertical, 4);
        copy.set_hexpand(true);
        let title = text(&track.title, "track-title");
        title.set_ellipsize(gtk::pango::EllipsizeMode::End);
        title.set_max_width_chars(50);
        title.set_tooltip_text(Some(&track.title));
        let channel = text(&track.channel, "muted");
        channel.add_css_class("small");
        channel.set_ellipsize(gtk::pango::EllipsizeMode::End);
        copy.append(&title);
        copy.append(&channel);
        row.append(&copy);
        let length = text(
            &track
                .duration
                .map(|d| clock(d as f64))
                .unwrap_or_else(|| "—".into()),
            "muted",
        );
        length.add_css_class("small");
        row.append(&length);
        let play = icon_button("media-playback-start-symbolic", "Phát bài này");
        play.add_css_class("flat");
        let h = self.handle.clone();
        let t = track.clone();
        play.connect_clicked(move |_| {
            h.send(if library {
                Command::PlayPlaylist(index)
            } else {
                Command::Play(t.clone())
            })
        });
        row.append(&play);
        if library {
            for (icon, delta, tip) in [
                ("go-up-symbolic", -1, "Đưa lên"),
                ("go-down-symbolic", 1, "Đưa xuống"),
            ] {
                let button = icon_button(icon, tip);
                button.add_css_class("flat");
                if delta == -1 && index == 0 {
                    button.set_sensitive(false);
                }
                let h = self.handle.clone();
                button.connect_clicked(move |_| h.send(Command::Move { index, delta }));
                row.append(&button);
            }
            let remove = icon_button("list-remove-symbolic", "Xóa khỏi playlist");
            remove.add_css_class("flat");
            let h = self.handle.clone();
            let id = track.id.clone();
            remove.connect_clicked(move |_| h.send(Command::Remove(id.clone())));
            row.append(&remove);
        } else {
            let add = icon_button("list-add-symbolic", "Thêm vào playlist đang chọn");
            add.add_css_class("accent");
            let h = self.handle.clone();
            let t = track.clone();
            add.connect_clicked(move |_| h.send(Command::Add(t.clone())));
            row.append(&add);
        }
        row
    }
    pub fn render(&self, snapshot: Snapshot) {
        self.updating.set(true);
        if self.rendered_revision.get() != Some(snapshot.revision) {
            clear(&self.playlists);
            for p in &snapshot.library.playlists {
                let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
                let name = text(&p.name, "track-title");
                name.set_ellipsize(gtk::pango::EllipsizeMode::End);
                name.set_max_width_chars(17);
                name.set_hexpand(true);
                row.append(&name);
                row.append(&text(&p.tracks.len().to_string(), "muted"));
                self.playlists.append(&row);
                if p.id == snapshot.library.selected
                    && let Some(row) = row.parent().and_downcast::<gtk::ListBoxRow>()
                {
                    self.playlists.select_row(Some(&row));
                }
            }
            clear(&self.tracks);
            if let Some(p) = snapshot.library.selected() {
                self.library_title.set_text(&p.name);
                self.library_info
                    .set_text(&format!("{} bài hát · lưu trên máy", p.tracks.len()));
                self.library_play.set_sensitive(!p.tracks.is_empty());
                for (i, track) in p.tracks.iter().enumerate() {
                    self.tracks.append(&self.track_row(track, i, true));
                }
            }
            self.rendered_revision.set(Some(snapshot.revision));
        }
        self.autoplay.set_active(snapshot.library.autoplay);
        self.volume.set_value(snapshot.library.volume);
        if let Some(track) = &snapshot.current {
            self.current_title.set_text(&track.title);
            self.current_artist.set_text(&track.channel);
        }
        self.play.set_sensitive(snapshot.current.is_some());
        self.play
            .set_icon_name(if snapshot.phase == Phase::Playing {
                "media-playback-pause-symbolic"
            } else {
                "media-playback-start-symbolic"
            });
        self.playback_status.set_text(&format!(
            "{}{}",
            snapshot.status,
            if snapshot.queue_remaining > 0 {
                format!(" · còn {} bài", snapshot.queue_remaining)
            } else {
                String::new()
            }
        ));
        self.seek.set_sensitive(
            matches!(snapshot.phase, Phase::Playing | Phase::Paused) && snapshot.duration > 0.0,
        );
        self.progress(snapshot.position, snapshot.duration);
        *self.state.borrow_mut() = Some(snapshot);
        self.updating.set(false);
    }
    pub fn progress(&self, pos: f64, duration: f64) {
        self.seek.set_range(0.0, duration.max(1.0));
        self.seek.set_value(pos);
        self.elapsed.set_text(&clock(pos));
        self.duration.set_text(&clock(duration));
        if let Some(state) = self.state.borrow_mut().as_mut() {
            state.position = pos;
            state.duration = duration;
        }
    }
    pub fn message(&self, message: &str) {
        self.message.set_text(message);
        self.message.set_visible(true);
        // One-shot timeout; does not wake the background player periodically.
        let label = self.message.downgrade();
        let expected = message.to_string();
        glib::timeout_add_local_once(std::time::Duration::from_secs(8), move || {
            if let Some(label) = label.upgrade()
                && label.text().as_str() == expected
            {
                label.set_visible(false);
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use may_music::engine::{self, UiEvent};
    use std::{
        fs,
        os::unix::fs::PermissionsExt,
        path::Path,
        time::{Duration, Instant},
    };

    #[track_caller]
    fn spin_until(mut predicate: impl FnMut() -> bool) {
        let context = glib::MainContext::default();
        let deadline = Instant::now() + Duration::from_secs(8);
        while !predicate() {
            assert!(Instant::now() < deadline, "UI test timed out");
            while context.pending() {
                context.iteration(false);
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }
    fn descendants(widget: &gtk::Widget) -> Vec<gtk::Widget> {
        let mut widgets = vec![];
        let mut child = widget.first_child();
        while let Some(w) = child {
            widgets.push(w.clone());
            widgets.extend(descendants(&w));
            child = w.next_sibling();
        }
        widgets
    }
    fn dbus(args: &[&str]) -> String {
        let mut child = std::process::Command::new("gdbus")
            .args(args)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        spin_until(|| child.try_wait().unwrap().is_some());
        let output = child.wait_with_output().unwrap();
        assert!(
            output.status.success(),
            "D-Bus failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout).unwrap()
    }

    #[test]
    #[ignore = "requires a display, session D-Bus and the offline fixture tools"]
    fn native_ui_search_add_play_hide_mpris_and_render() {
        let temp = tempfile::tempdir().unwrap();
        let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
        for name in ["yt-dlp.py", "mpv.py"] {
            fs::set_permissions(fixtures.join(name), fs::Permissions::from_mode(0o755)).unwrap();
        }
        unsafe {
            std::env::set_var("MAY_TEST_DIR", temp.path());
            std::env::set_var("MAY_DATA_DIR", temp.path().join("data"));
            std::env::set_var("MAY_YTDLP", fixtures.join("yt-dlp.py"));
            std::env::set_var("MAY_MPV", fixtures.join("mpv.py"));
        }
        let app = gtk::Application::builder()
            .application_id("io.github.maymusic.UiTest")
            .flags(gio::ApplicationFlags::NON_UNIQUE)
            .build();
        app.register(gio::Cancellable::NONE).unwrap();
        let (handle, events, worker) = engine::launch().unwrap();
        let state = Rc::new(RefCell::new(None));
        let ui = Ui::new(&app, handle.clone(), state.clone());
        let mpris = crate::mpris::Mpris::new(&app, state.clone(), &handle).unwrap();
        let target = ui.clone();
        glib::spawn_future_local(async move {
            while let Ok(event) = events.recv().await {
                match event {
                    UiEvent::State(snapshot) => target.render(*snapshot),
                    UiEvent::SearchResults(tracks) => target.results(tracks),
                    UiEvent::Progress(p, d) => target.progress(p, d),
                    UiEvent::SearchStarted => target.searching(),
                    UiEvent::SearchError(e) | UiEvent::Message(e) => target.message(&e),
                }
            }
        });
        ui.window.present();
        spin_until(|| state.borrow().is_some());
        let search = descendants(ui.window.upcast_ref())
            .into_iter()
            .filter_map(|w| w.downcast::<gtk::Entry>().ok())
            .find(|e| {
                e.placeholder_text()
                    .is_some_and(|p| p.starts_with("Tên bài hát"))
            })
            .unwrap();
        search.set_text("nhạc buổi sáng");
        search.emit_activate();
        spin_until(|| ui.results.row_at_index(1).is_some());
        for i in 0..2 {
            let row = ui.results.row_at_index(i).unwrap();
            let add = descendants(row.upcast_ref())
                .into_iter()
                .filter_map(|w| w.downcast::<gtk::Button>().ok())
                .find(|b| b.tooltip_text().is_some_and(|t| t.starts_with("Thêm vào")))
                .unwrap();
            add.emit_clicked();
            spin_until(|| {
                state
                    .borrow()
                    .as_ref()
                    .unwrap()
                    .library
                    .selected()
                    .unwrap()
                    .tracks
                    .len()
                    == (i + 1) as usize
            });
        }
        search.set_text("https://youtu.be/music000002?si=shared-link");
        search.emit_activate();
        spin_until(|| {
            ui.search_info.text().starts_with("1 KẾT QUẢ") && ui.results.row_at_index(1).is_none()
        });
        let linked_row = ui.results.row_at_index(0).unwrap();
        assert!(
            descendants(linked_row.upcast_ref())
                .into_iter()
                .filter_map(|w| w.downcast::<gtk::Label>().ok())
                .any(|label| label.text() == "Bài từ link YouTube")
        );
        ui.library_play.emit_clicked();
        spin_until(|| state.borrow().as_ref().unwrap().phase == Phase::Playing);
        let status = dbus(&[
            "call",
            "--session",
            "--dest",
            "org.mpris.MediaPlayer2.may_music",
            "--object-path",
            "/org/mpris/MediaPlayer2",
            "--method",
            "org.freedesktop.DBus.Properties.Get",
            "org.mpris.MediaPlayer2.Player",
            "PlaybackStatus",
        ]);
        assert!(status.contains("Playing"));
        dbus(&[
            "call",
            "--session",
            "--dest",
            "org.mpris.MediaPlayer2.may_music",
            "--object-path",
            "/org/mpris/MediaPlayer2",
            "--method",
            "org.mpris.MediaPlayer2.Player.Pause",
        ]);
        spin_until(|| state.borrow().as_ref().unwrap().phase == Phase::Paused);
        handle.send(Command::Toggle);
        spin_until(|| state.borrow().as_ref().unwrap().phase == Phase::Playing);
        ui.window.close();
        spin_until(|| !ui.window.is_visible());
        assert_eq!(state.borrow().as_ref().unwrap().phase, Phase::Playing);
        fs::write(temp.path().join("player-event"), "eof").unwrap();
        spin_until(|| {
            state
                .borrow()
                .as_ref()
                .unwrap()
                .current
                .as_ref()
                .unwrap()
                .id
                == "music000002"
                && state.borrow().as_ref().unwrap().phase == Phase::Playing
        });
        ui.window.present();
        if let Some(path) = std::env::var_os("MAY_TEST_SCREENSHOT") {
            let start = Instant::now();
            spin_until(|| start.elapsed() > Duration::from_millis(500));
            let paintable = gtk::WidgetPaintable::new(Some(&ui.window));
            let snapshot = gtk::Snapshot::new();
            paintable.snapshot(
                &snapshot,
                ui.window.width() as f64,
                ui.window.height() as f64,
            );
            let node = snapshot.to_node().unwrap();
            let texture = ui.window.renderer().unwrap().render_texture(&node, None);
            texture.save_to_png(path).unwrap();
        }
        drop(mpris);
        ui.window.destroy();
        drop(worker);
    }
}
