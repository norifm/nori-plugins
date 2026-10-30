//! Finds nori speakers over mDNS and offers them in nori's devices list; picking one casts to it as a
//! deep sink. The host speaks the protocol, from the first frame to the buttons
//! (docs/developers/plugins.md in nori, "Deep sinks").

use nori_plugin_sdk::audio_out::{self, Encoding, Pace};
use nori_plugin_sdk::cast::{self, Kind, Target};
use nori_plugin_sdk::hooks::Event;
use nori_plugin_sdk::sockets::Tcp;
use nori_plugin_sdk::types::Service;
use nori_plugin_sdk::{discovery, log, Hook, Hooks, Lifecycle, Manifest};

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
    /// The speakers found, by their mDNS name.
    found: Vec<Service>,
    /// What a cast keeps open while it runs.
    casting: Option<Tcp>,
}

impl Speaker {
    fn offer(&self) {
        cast::offer(&self.found.iter().map(|s| Target { id: s.name.clone(), name: name_of(s) }).collect::<Vec<_>>());
    }

    fn start(&mut self, id: &str) {
        self.stop();
        let Some(found) = self.found.iter().find(|s| s.name == id).cloned() else { return };
        let started = Tcp::connect(&found.host, found.port, CONNECT_MS).and_then(|tcp| {
            audio_out::to_socket(&tcp, Encoding::Pcm16, None, None, Pace::Deep(Some(DEPTH_MS)))?;
            Ok(tcp)
        });
        match started {
            Ok(tcp) => {
                self.casting = Some(tcp);
                cast::start(Kind::Sink, &name_of(&found));
            }
            Err(e) => log::write(log::Level::Warn, &format!("{}: {:?}", found.name, e.kind)),
        }
    }

    fn stop(&mut self) {
        if self.casting.take().is_some() {
            audio_out::stop();
            cast::end();
        }
    }
}

impl Lifecycle for Speaker {
    fn action(&mut self, id: String) {
        if let Some(speaker) = id.strip_prefix("cast:") {
            self.start(speaker);
        }
    }
}

impl Hooks for Speaker {
    fn on(&mut self, event: Event) {
        match event {
            // Looks for speakers only while a picker could list them.
            Event::OutputPicker(true) => {
                self.found.clear();
                self.offer();
                discovery::browse(KIND);
            }
            Event::OutputPicker(false) => discovery::stop(KIND),
            Event::Discovered(service) => {
                self.found.retain(|s| s.name != service.name);
                self.found.push(service);
                self.offer();
            }
            Event::MediaButton(button) if button == "cast-stop" => self.stop(),
            _ => {}
        }
    }
}

const MANIFEST: Manifest = Manifest { net: &["lan"], hooks: &[Hook::OutputPicker, Hook::MediaButton], ..Manifest::new("fm.nori.speaker") };

nori_plugin_sdk::plugin!(Speaker, MANIFEST, lifecycle, hooks);
