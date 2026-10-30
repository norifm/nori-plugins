//! Finds nori speakers over mDNS and casts to one as a deep sink: the host speaks the protocol, from
//! the first frame to the buttons (docs/developers/plugins.md in nori, "Deep sinks").

use nori_plugin_sdk::audio_out::{self, Encoding, Pace};
use nori_plugin_sdk::cast::{self, Kind};
use nori_plugin_sdk::hooks::Event;
use nori_plugin_sdk::sockets::Tcp;
use nori_plugin_sdk::types::Service;
use nori_plugin_sdk::{discovery, log, rows, Hook, Hooks, Lifecycle, Manifest, Place, Row, RowKind, Slots};

/// What the speakers announce.
const KIND: &str = "_nori-speaker._tcp";

/// How much a speaker is asked to hold, ms.
const DEPTH_MS: u32 = 60_000;

const CONNECT_MS: u32 = 3000;

/// What the user calls `service`: the name it says in its TXT, else its own.
fn name_of(service: &Service) -> String {
    service.txt.iter().find(|(key, _)| key == "name").map_or_else(|| service.name.clone(), |(_, name)| name.clone())
}

#[derive(Default)]
struct Speaker {
    /// The speaker last found.
    found: Option<Service>,
    /// What a cast keeps open while it runs.
    casting: Option<Tcp>,
}

impl Speaker {
    fn say(&self, words: &str, name: &str) {
        rows::set_status("speaker", words, &[name.to_string()]);
    }

    fn start(&mut self) {
        let Some(found) = self.found.clone() else { return };
        let name = name_of(&found);
        let started = Tcp::connect(&found.host, found.port, CONNECT_MS).and_then(|tcp| {
            audio_out::to_socket(&tcp, Encoding::Pcm16, None, None, Pace::Deep(Some(DEPTH_MS)))?;
            Ok(tcp)
        });
        match started {
            Ok(tcp) => {
                self.casting = Some(tcp);
                cast::start(Kind::Sink, &name);
                self.say("casting", &name);
            }
            Err(e) => {
                log::write(log::Level::Warn, &format!("{name}: {:?}", e.kind));
                self.say("unreachable", &name);
            }
        }
    }

    fn stop(&mut self) {
        cast::end();
        self.casting = None;
        match &self.found {
            Some(found) => self.say("found", &name_of(found)),
            None => self.say("searching", ""),
        }
    }
}

impl Lifecycle for Speaker {
    fn action(&mut self, id: String) {
        if id == "connect" {
            if self.casting.is_some() {
                self.stop();
            } else {
                self.start();
            }
        }
    }
}

impl Hooks for Speaker {
    fn on(&mut self, event: Event) {
        match event {
            // Looks for speakers only while a picker could list them.
            Event::OutputPicker(true) => {
                self.say("searching", "");
                discovery::browse(KIND);
            }
            Event::OutputPicker(false) => discovery::stop(KIND),
            Event::Discovered(service) => {
                if self.casting.is_none() {
                    self.say("found", &name_of(&service));
                }
                self.found = Some(service);
            }
            Event::MediaButton(button) if button == "cast-stop" => self.stop(),
            _ => {}
        }
    }
}

/// Its rows on the devices sheet are all it fills.
impl Slots for Speaker {}

const MANIFEST: Manifest = Manifest {
    net: &["lan"],
    rows: &[Row::new("speaker", RowKind::Status), Row::new("connect", RowKind::Action)],
    places: &[Place::Devices],
    hooks: &[Hook::OutputPicker, Hook::MediaButton],
    ..Manifest::new("fm.nori.speaker")
};

nori_plugin_sdk::plugin!(Speaker, MANIFEST, lifecycle, hooks, slots);
