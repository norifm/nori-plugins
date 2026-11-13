//! Which of the Google account's channels a profile is served as: its own, or the one chosen on the
//! profile's page, kept in the store under the profile's id.

use nori_plugin_sdk::provider::Identity;
use nori_plugin_sdk::types::{Error, ErrorKind};
use serde::{Deserialize, Serialize};

use crate::host::{block, Host, Wire};
use crate::YouTube;

/// A channel chosen: what requests name (none for the account's own).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Chosen {
    pub(crate) page_id: Option<String>,
}

fn key(profile: &str) -> String {
    format!("{profile}/channel")
}

/// Profile `profile`'s channel as kept; None when another was never chosen.
pub(crate) fn chosen(host: &impl Host, profile: &str) -> Option<Chosen> {
    host.get(&key(profile)).and_then(|j| serde_json::from_slice(&j).ok())
}

impl<H: Host + Sync> YouTube<H> {
    /// The account's channels, its own first, the one served now chosen; kept for [`Self::choose_channel`].
    pub(crate) fn channels(&mut self) -> Result<Vec<Identity>, Error> {
        self.fresh()?;
        if !self.signed_in() {
            return Ok(Vec::new());
        }
        let r = block(self.open.api.channels(&Wire(&self.host)));
        let mut list = self.after(r)?;
        list.sort_by_key(|c| c.page_id.is_some());
        let now = chosen(&self.host, &self.open.profile).and_then(|c| c.page_id);
        let shown = list.iter().map(|c| Identity { name: c.name.clone(), detail: c.handle.clone(), picture: c.thumbnail.as_deref().map(crate::cover), chosen: c.page_id == now }).collect();
        self.open.channels = list;
        Ok(shown)
    }

    /// The picture of the channel served now; the channels are listed first when they were not yet.
    pub(crate) fn channel_picture(&mut self) -> Option<String> {
        if self.open.channels.is_empty() {
            self.channels().ok()?;
        }
        let now = chosen(&self.host, &self.open.profile).and_then(|c| c.page_id);
        self.open.channels.iter().find(|c| c.page_id == now)?.thumbnail.as_deref().map(crate::cover)
    }

    /// Serves the profile as [`Self::channels`]' entry `index` from now on.
    pub(crate) fn choose_channel(&mut self, index: u32) -> Result<(), Error> {
        let Some(c) = self.open.channels.get(index as usize) else {
            return Err(Error { kind: ErrorKind::NotFound, detail: format!("no channel {index} listed") });
        };
        let page_id = c.page_id.clone();
        match &page_id {
            Some(_) => self.host.set(&key(&self.open.profile), &serde_json::to_vec(&Chosen { page_id: page_id.clone() }).unwrap_or_default()),
            None => self.host.remove(&key(&self.open.profile)),
        }
        self.open.api.set_channel(page_id);
        Ok(())
    }

    /// Serves the profile as its account's own channel again.
    pub(crate) fn forget_channel(&mut self) {
        self.host.remove(&key(&self.open.profile));
        self.open.channels.clear();
        self.open.api.set_channel(None);
    }
}
