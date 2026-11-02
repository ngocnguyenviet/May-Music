use gtk::{gio, glib, prelude::*};
use may_music::engine::{Command, Handle, Phase, Snapshot};
use std::{cell::RefCell, collections::HashMap, rc::Rc};

const PATH: &str = "/org/mpris/MediaPlayer2";
const PLAYER: &str = "org.mpris.MediaPlayer2.Player";
const XML: &str = r#"<node>
<interface name="org.mpris.MediaPlayer2">
 <method name="Raise"/><method name="Quit"/>
 <property name="CanQuit" type="b" access="read"/>
 <property name="CanRaise" type="b" access="read"/>
 <property name="HasTrackList" type="b" access="read"/>
 <property name="Identity" type="s" access="read"/>
 <property name="DesktopEntry" type="s" access="read"/>
 <property name="SupportedUriSchemes" type="as" access="read"/>
 <property name="SupportedMimeTypes" type="as" access="read"/>
</interface>
<interface name="org.mpris.MediaPlayer2.Player">
 <method name="Next"/><method name="Previous"/><method name="Pause"/>
 <method name="PlayPause"/><method name="Stop"/><method name="Play"/>
 <method name="Seek"><arg name="Offset" type="x" direction="in"/></method>
 <method name="SetPosition"><arg name="TrackId" type="o" direction="in"/><arg name="Position" type="x" direction="in"/></method>
 <signal name="Seeked"><arg name="Position" type="x"/></signal>
 <property name="PlaybackStatus" type="s" access="read"/>
 <property name="Rate" type="d" access="readwrite"/>
 <property name="MinimumRate" type="d" access="read"/>
 <property name="MaximumRate" type="d" access="read"/>
 <property name="Metadata" type="a{sv}" access="read"/>
 <property name="Volume" type="d" access="readwrite"/>
 <property name="Position" type="x" access="read"/>
 <property name="CanGoNext" type="b" access="read"/>
 <property name="CanGoPrevious" type="b" access="read"/>
 <property name="CanPlay" type="b" access="read"/>
 <property name="CanPause" type="b" access="read"/>
 <property name="CanSeek" type="b" access="read"/>
 <property name="CanControl" type="b" access="read"/>
</interface></node>"#;

fn track_path(s: &Snapshot) -> glib::variant::ObjectPath {
    let id = s
        .current
        .as_ref()
        .map(|t| t.id.replace('-', "_"))
        .unwrap_or_else(|| "none".into());
    glib::variant::ObjectPath::try_from(format!("/io/github/maymusic/track/{id}")).unwrap()
}

fn property(s: Option<&Snapshot>, name: &str) -> glib::Variant {
    match name {
        "CanQuit" | "CanRaise" | "CanControl" => true.to_variant(),
        "HasTrackList" => false.to_variant(),
        "Identity" => "Mây Music".to_variant(),
        "DesktopEntry" => "io.github.maymusic.Player".to_variant(),
        "SupportedUriSchemes" | "SupportedMimeTypes" => Vec::<String>::new().to_variant(),
        "PlaybackStatus" => match s.map(|s| s.phase) {
            Some(Phase::Playing) => "Playing",
            Some(Phase::Paused) => "Paused",
            _ => "Stopped",
        }
        .to_variant(),
        "Rate" | "MinimumRate" | "MaximumRate" => 1.0f64.to_variant(),
        "Volume" => s
            .map(|s| s.library.volume / 100.0)
            .unwrap_or(0.7)
            .to_variant(),
        "Position" => s
            .map(|s| (s.position * 1_000_000.0) as i64)
            .unwrap_or(0)
            .to_variant(),
        "CanGoNext" | "CanGoPrevious" | "CanPlay" => {
            s.is_some_and(|s| s.current.is_some()).to_variant()
        }
        "CanPause" => s.is_some_and(|s| s.phase != Phase::Stopped).to_variant(),
        "CanSeek" => s
            .is_some_and(|s| matches!(s.phase, Phase::Playing | Phase::Paused) && s.duration > 0.0)
            .to_variant(),
        "Metadata" => {
            let mut metadata = HashMap::<String, glib::Variant>::new();
            if let Some(s) = s
                && let Some(track) = &s.current
            {
                metadata.insert("mpris:trackid".into(), track_path(s).to_variant());
                metadata.insert(
                    "mpris:length".into(),
                    ((s.duration * 1_000_000.0) as i64).to_variant(),
                );
                metadata.insert("xesam:title".into(), track.title.to_variant());
                metadata.insert(
                    "xesam:artist".into(),
                    vec![track.channel.clone()].to_variant(),
                );
                metadata.insert("xesam:url".into(), track.url().to_variant());
            }
            metadata.to_variant()
        }
        _ => false.to_variant(),
    }
}

pub struct Mpris {
    connection: gio::DBusConnection,
    registrations: Vec<gio::RegistrationId>,
    owner: Option<gio::OwnerId>,
}

impl Mpris {
    pub fn new(
        app: &gtk::Application,
        state: Rc<RefCell<Option<Snapshot>>>,
        handle: &Handle,
    ) -> Result<Self, glib::Error> {
        let connection = gio::bus_get_sync(gio::BusType::Session, gio::Cancellable::NONE)?;
        let info = gio::DBusNodeInfo::for_xml(XML)?;
        let mut registrations = Vec::new();
        for interface in ["org.mpris.MediaPlayer2", PLAYER] {
            let app = app.downgrade();
            let reader = state.clone();
            let track_state = state.clone();
            let handle = handle.clone();
            let setter = handle.clone();
            let registration = connection
                .register_object(PATH, &info.lookup_interface(interface).unwrap())
                .method_call(move |connection, _, _, _, method, params, invocation| {
                    let command = match method {
                        "Raise" => {
                            if let Some(app) = app.upgrade() {
                                app.activate();
                            }
                            None
                        }
                        "Quit" => {
                            if let Some(app) = app.upgrade() {
                                app.quit();
                            }
                            None
                        }
                        "Next" => Some(Command::Next),
                        "Previous" => Some(Command::Previous),
                        "Pause" => Some(Command::Pause(true)),
                        "Play" => Some(Command::Pause(false)),
                        "PlayPause" => Some(Command::Toggle),
                        "Stop" => Some(Command::Stop),
                        "Seek" => {
                            let offset = params.child_get::<i64>(0) as f64 / 1_000_000.0;
                            let target = track_state
                                .borrow()
                                .as_ref()
                                .map(|s| (s.position + offset).clamp(0.0, s.duration.max(0.0)));
                            if let Some(target) = target {
                                let _ = connection.emit_signal(
                                    None,
                                    PATH,
                                    PLAYER,
                                    "Seeked",
                                    Some(&((target * 1_000_000.0) as i64,).to_variant()),
                                );
                            }
                            Some(Command::SeekRelative(offset))
                        }
                        "SetPosition" => {
                            let id = params.child_get::<glib::variant::ObjectPath>(0);
                            let position = params.child_get::<i64>(1) as f64 / 1_000_000.0;
                            if track_state.borrow().as_ref().is_some_and(|s| {
                                track_path(s) == id && position >= 0.0 && position <= s.duration
                            }) {
                                let _ = connection.emit_signal(
                                    None,
                                    PATH,
                                    PLAYER,
                                    "Seeked",
                                    Some(&((position * 1_000_000.0) as i64,).to_variant()),
                                );
                                Some(Command::Seek(position))
                            } else {
                                None
                            }
                        }
                        _ => {
                            invocation.return_dbus_error(
                                "org.freedesktop.DBus.Error.NotSupported",
                                "Method not supported",
                            );
                            return;
                        }
                    };
                    if let Some(command) = command {
                        handle.send(command);
                    }
                    invocation.return_value(None);
                })
                .property(move |_, _, _, _, name| property(reader.borrow().as_ref(), name))
                .set_property(move |_, _, _, _, name, value| match name {
                    "Volume" => {
                        if let Some(v) = value.get::<f64>().filter(|v| v.is_finite()) {
                            setter.send(Command::Volume(v * 100.0));
                            true
                        } else {
                            false
                        }
                    }
                    "Rate" => value.get::<f64>() == Some(1.0),
                    _ => false,
                })
                .build()?;
            registrations.push(registration);
        }
        let owner = gio::bus_own_name_on_connection(
            &connection,
            "org.mpris.MediaPlayer2.may_music",
            gio::BusNameOwnerFlags::NONE,
            |_, _| {},
            |_, _| {},
        );
        Ok(Self {
            connection,
            registrations,
            owner: Some(owner),
        })
    }
    pub fn changed(&self, s: &Snapshot) {
        let mut changed = HashMap::<String, glib::Variant>::new();
        for name in [
            "PlaybackStatus",
            "Metadata",
            "Volume",
            "CanGoNext",
            "CanGoPrevious",
            "CanPlay",
            "CanPause",
            "CanSeek",
        ] {
            changed.insert(name.into(), property(Some(s), name));
        }
        let params = (PLAYER, changed, Vec::<String>::new()).to_variant();
        let _ = self.connection.emit_signal(
            None,
            PATH,
            "org.freedesktop.DBus.Properties",
            "PropertiesChanged",
            Some(&params),
        );
    }
}

impl Drop for Mpris {
    fn drop(&mut self) {
        for id in self.registrations.drain(..) {
            let _ = self.connection.unregister_object(id);
        }
        if let Some(owner) = self.owner.take() {
            gio::bus_unown_name(owner);
        }
    }
}
