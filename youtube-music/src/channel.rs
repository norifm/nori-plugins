//! Which of the Google account's channels a profile is served as: its own, or the one last switched to
//! on the plugin's page, kept in the store under the profile's id.

use innertube_music as yt;
use serde::{Deserialize, Serialize};

use crate::host::{block, Host, Wire};
use crate::YouTube;

/// The row saying which channel the library is.
pub(crate) const CHANNEL: &str = "channel";
/// The row switching to the account's next channel.
pub(crate) const NEXT: &str = "next_channel";

/// A channel switched to: what requests name (none for the account's own), and its name.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Chosen {
    pub(crate) page_id: Option<String>,
    pub(crate) name: String,
}

fn key(profile: &str) -> String {
    format!("{profile}/channel")
}

/// Profile `profile`'s channel as kept; None when it was never switched.
pub(crate) fn chosen(host: &impl Host, profile: &str) -> Option<Chosen> {
    host.get(&key(profile)).and_then(|j| serde_json::from_slice(&j).ok())
}

impl<H: Host + Sync> YouTube<H> {
    /// The page's channel rows: there while signed in, saying which channel the library is.
    pub(crate) fn show_channel(&self) {
        let on = self.signed_in();
        self.host.show(CHANNEL, on);
        self.host.show(NEXT, on);
        if !on {
            return;
        }
        match chosen(&self.host, &self.open.profile) {
            Some(c) => self.host.status(CHANNEL, "channel_is", &[c.name]),
            None => self.host.status(CHANNEL, "channel_own", &[]),
        }
    }

    /// Serves the profile as its account's own channel again.
    pub(crate) fn forget_channel(&mut self) {
        self.host.remove(&key(&self.open.profile));
        self.open.api.set_channel(None);
    }

    /// Serves the profile as the account's channel after the one served now, round to the first.
    pub(crate) fn next_channel(&mut self) {
        if self.fresh().is_err() || !self.signed_in() {
            return;
        }
        let r = block(self.open.api.channels(&Wire(&self.host)));
        let list = match self.after(r) {
            Ok(list) if !list.is_empty() => list,
            Ok(_) => return,
            Err(e) => return self.host.log(&format!("YouTube channels: not listed: {}", e.detail)),
        };
        let now = chosen(&self.host, &self.open.profile).and_then(|c| c.page_id);
        let at = list.iter().position(|c| c.page_id == now).map_or(0, |i| (i + 1) % list.len());
        let next: &yt::Channel = &list[at];
        let kept = Chosen { page_id: next.page_id.clone(), name: next.name.clone() };
        self.host.set(&key(&self.open.profile), &serde_json::to_vec(&kept).unwrap_or_default());
        self.open.api.set_channel(kept.page_id);
        self.show_channel();
    }
}
