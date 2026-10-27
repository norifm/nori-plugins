//! A profile's account: a browser session's cookies, kept in the plugin's store under the profile's id
//! and replaced there the moment YouTube rotates them, rotated before they go stale whenever the
//! profile is used, and given up only when YouTube refuses them.

use innertube_music as yt;
use nori_plugin_sdk::provider::{AccountState, SignInResult};
use nori_plugin_sdk::types::{Error, ErrorKind};
use serde::{Deserialize, Serialize};

use crate::host::{block, Host, Wire};
use crate::{error, YouTube};

/// The session as kept.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Kept {
    /// The `Cookie` header's value.
    pub(crate) cookies: String,
    /// When the cookies were last rotated (Unix seconds).
    pub(crate) rotated_at: u64,
    pub(crate) ended: bool,
    /// Ended, and asked not to be reminded of at start.
    #[serde(default)]
    pub(crate) quiet: bool,
}

/// The store key of profile `profile`'s session.
fn session_key(profile: &str) -> String {
    format!("{profile}/session")
}

/// The store key of profile `profile`'s visitor id: the same visitor is served the same Home.
fn visitor_key(profile: &str) -> String {
    format!("{profile}/visitor")
}

/// Profile `profile`'s session as kept in the store.
pub(crate) fn stored(host: &impl Host, profile: &str) -> Option<Kept> {
    host.get(&session_key(profile)).and_then(|j| serde_json::from_slice(&j).ok())
}

pub(crate) fn unix_now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

impl<H: Host + Sync> YouTube<H> {
    /// innertube-music's session for `profile`: its kept visitor id and, signed in, its cookies.
    pub(crate) fn load(&mut self, profile: String) {
        let api = yt::InnerTube::default();
        api.set_visitor(self.host.get(&visitor_key(&profile)).and_then(|v| String::from_utf8(v).ok()));
        let kept = stored(&self.host, &profile);
        if let Some(k) = kept.as_ref().filter(|k| !k.ended) {
            api.set_cookies(Some(yt::Cookies::parse(&k.cookies)));
        }
        self.open = crate::Open { profile, api, kept, ..Default::default() };
    }

    /// Keeps `kept` in memory and in the store at once; None forgets the session.
    fn keep(&mut self, kept: Option<Kept>) {
        let key = session_key(&self.open.profile);
        match &kept {
            Some(k) => self.host.set(&key, &serde_json::to_vec(k).unwrap_or_default()),
            None => self.host.remove(&key),
        }
        self.open.kept = kept;
    }

    /// Before a call: a signed-in session last rotated [`yt::auth::ROTATE_EVERY`] ago or more is rotated
    /// first, unless another instance using the store has rotated it since; a failure that is not a
    /// refusal leaves it for the next call. An ended session is asked about once a profile opening, and
    /// taken up again if YouTube still knows its cookies.
    /// [`YouTube::fresh_session`] before a call; a session it ends fails the call as unauthorized, so
    /// nori asks for the account's rules anew.
    pub(crate) fn fresh(&mut self) -> Result<(), Error> {
        let was = self.signed_in();
        self.fresh_session(unix_now());
        if was && !self.signed_in() {
            return Err(Error { kind: ErrorKind::Unauthorized, detail: "YouTube Music ended the session".into() });
        }
        Ok(())
    }

    fn fresh_session(&mut self, now: u64) {
        let Some(mut kept) = self.open.kept.clone() else { return };
        if kept.ended {
            return self.recheck(kept);
        }
        if !yt::auth::rotation_due(kept.rotated_at, now) {
            return;
        }
        if let Some(newer) = stored(&self.host, &self.open.profile).filter(|l| *l != kept) {
            self.open.api.set_cookies((!newer.ended).then(|| yt::Cookies::parse(&newer.cookies)));
            self.open.kept = Some(newer.clone());
            if newer.ended || !yt::auth::rotation_due(newer.rotated_at, now) {
                return;
            }
            kept = newer;
        }
        match block(self.open.api.rotate(&Wire(&self.host))) {
            Ok(_) => {
                self.open.api.take_changed();
                let cookies = self.open.api.cookies().map_or(kept.cookies, |c| c.header());
                self.keep(Some(Kept { cookies, rotated_at: now, ended: false, quiet: false }));
            }
            Err(e) => {
                let ended = matches!(e, yt::Error::SignedOut);
                self.host.log(&format!("YouTube session: not rotated: {}", error(e).detail));
                if ended {
                    self.keep(Some(Kept { ended: true, ..kept }));
                }
            }
        }
    }

    /// An ended session's cookies asked about once: still the account's, it is signed in again.
    fn recheck(&mut self, kept: Kept) {
        if std::mem::replace(&mut self.open.rechecked, true) {
            return;
        }
        // Asked apart: the profile's own calls stay a visitor's unless YouTube takes the cookies.
        let probe = yt::InnerTube::default();
        probe.set_cookies(Some(yt::Cookies::parse(&kept.cookies)));
        match block(probe.account_name(&Wire(&self.host))) {
            Ok(_) => {
                let cookies = probe.cookies().map_or(kept.cookies, |c| c.header());
                self.open.api.set_cookies(Some(yt::Cookies::parse(&cookies)));
                self.keep(Some(Kept { cookies, rotated_at: unix_now(), ended: false, quiet: false }));
            }
            Err(e) => self.host.log(&format!("YouTube session: still ended: {}", error(e).detail)),
        }
    }

    /// After a call: a visitor id and cookies YouTube gave are kept at once; a session it refused is
    /// marked ended, with the cookies it last had.
    pub(crate) fn after<T>(&mut self, r: yt::Result<T, Error>) -> Result<T, Error> {
        if let Some(visitor) = self.open.api.take_new_visitor() {
            self.host.set(&visitor_key(&self.open.profile), visitor.as_bytes());
        }
        if let Some(kept) = self.open.kept.clone().filter(|k| !k.ended) {
            if matches!(r, Err(yt::Error::SignedOut)) {
                let cookies = self.open.api.cookies().map_or(kept.cookies, |c| c.header());
                self.open.api.set_cookies(None);
                self.keep(Some(Kept { cookies, ended: true, ..kept }));
            } else if let Some(changed) = self.open.api.take_changed() {
                self.keep(Some(Kept { cookies: changed.header(), ..kept }));
            }
        }
        r.map_err(error)
    }

    /// Whether the session's cookies go with the requests.
    pub(crate) fn signed_in(&self) -> bool {
        self.open.kept.as_ref().is_some_and(|k| !k.ended)
    }

    /// Signs in with a browser session's cookies (a `Cookie` header or a cookies.txt), checked by asking
    /// YouTube for the account's name.
    pub(crate) fn sign_in_with(&mut self, cookies: String) -> SignInResult {
        let jar = yt::Cookies::parse(&cookies);
        if !jar.signed_in() {
            return SignInResult::NoSession;
        }
        let before = self.open.api.cookies();
        self.open.api.set_cookies(Some(jar));
        match block(self.open.api.account_name(&Wire(&self.host))) {
            Ok(name) => {
                let cookies = self.open.api.cookies().map(|c| c.header()).unwrap_or_default();
                self.open.api.take_changed();
                // Fresh from the browser: it was just rotated.
                self.keep(Some(Kept { cookies, rotated_at: unix_now(), ended: false, quiet: false }));
                SignInResult::SignedIn(name)
            }
            Err(e) => {
                self.open.api.set_cookies(before);
                match e {
                    yt::Error::SignedOut | yt::Error::Refused | yt::Error::Status(401 | 403) => SignInResult::Refused,
                    e => SignInResult::Unreachable(error(e).detail),
                }
            }
        }
    }

    /// Forgets the session's cookies here.
    pub(crate) fn forget_session(&mut self) {
        self.open.api.set_cookies(None);
        self.keep(None);
    }

    /// An ended session not yet asked to be quiet: the client reminds to sign in again at start.
    pub(crate) fn reminds_to_sign_in(&self) -> bool {
        self.open.kept.as_ref().is_some_and(|k| k.ended && !k.quiet)
    }

    /// Stops the reminder until the account is signed in, and its session ends, again.
    pub(crate) fn quiet_reminder(&mut self) {
        if let Some(k) = self.open.kept.clone().filter(|k| k.ended) {
            self.keep(Some(Kept { quiet: true, ..k }));
        }
    }

    pub(crate) fn account_state(&self) -> AccountState {
        match &self.open.kept {
            None => AccountState::SignedOut,
            Some(k) if k.ended => AccountState::Ended,
            Some(_) => AccountState::SignedIn,
        }
    }
}

