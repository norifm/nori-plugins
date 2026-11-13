//! What the plugin asks of nori, behind [`Host`] so tests answer it with fixtures.

use std::future::Future;

use innertube_music as yt;
use nori_plugin_sdk::types::{Error, Header};
use nori_plugin_sdk::{fetching, http, log, store};

/// nori's HTTP, the plugin's store and its log.
pub(crate) trait Host {
    fn fetch(&self, request: yt::Request) -> Result<yt::Response, Error>;
    /// [`Host::fetch`] on its way beside others polled with it.
    fn send(&self, request: yt::Request) -> impl Future<Output = Result<yt::Response, Error>> + Send {
        std::future::ready(self.fetch(request))
    }
    fn get(&self, key: &str) -> Option<Vec<u8>>;
    fn set(&self, key: &str, value: &[u8]);
    fn remove(&self, key: &str);
    fn log(&self, line: &str);
}

/// The host's imports.
#[derive(Default)]
pub(crate) struct Nori;

/// A body goes as a JSON POST, as innertube-music sends every one.
fn request(r: yt::Request) -> http::Request {
    let method = if r.body.is_some() { http::Method::Post } else { http::Method::Get };
    let headers = r.headers.into_iter().map(|(name, value)| Header { name, value }).collect();
    http::Request { method, url: r.url, headers, body: r.body.map(String::into_bytes), timeout_ms: None }
}

fn response(a: http::Response) -> yt::Response {
    let set_cookies = a.headers.into_iter().filter(|h| h.name.eq_ignore_ascii_case("set-cookie")).map(|h| h.value).collect();
    yt::Response { status: a.status, body: a.body, set_cookies }
}

impl Host for Nori {
    fn fetch(&self, r: yt::Request) -> Result<yt::Response, Error> {
        http::fetch(&request(r)).map(response)
    }

    fn send(&self, r: yt::Request) -> impl Future<Output = Result<yt::Response, Error>> + Send {
        let sent = fetching::send(request(r));
        async move { sent.await.map(response) }
    }

    fn get(&self, key: &str) -> Option<Vec<u8>> {
        store::get(key)
    }

    fn set(&self, key: &str, value: &[u8]) {
        if let Err(e) = store::set(key, value) {
            self.log(&format!("could not keep {key}: {}", e.detail));
        }
    }

    fn remove(&self, key: &str) {
        store::remove(key)
    }

    fn log(&self, line: &str) {
        log::write(log::Level::Info, line)
    }
}

/// The host's HTTP as innertube-music's client: the requests it asks for together are on their way at
/// once.
pub(crate) struct Wire<'a, H>(pub(crate) &'a H);

impl<H: Host + Sync> yt::Http for Wire<'_, H> {
    type Error = Error;

    fn send(&self, request: yt::Request) -> impl Future<Output = Result<yt::Response, Error>> + Send {
        self.0.send(request)
    }
}

/// `f`'s answer, its requests waited for together.
pub(crate) fn block<F: Future>(f: F) -> F::Output {
    fetching::run(f)
}
