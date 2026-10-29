//! What the plugin asks of nori, behind [`Host`] so tests answer it with fixtures.

use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};

use nori_plugin_sdk::types::Error;
use nori_plugin_sdk::{fetching, http, log, store};

/// nori's HTTP, the plugin's store and its log.
pub(crate) trait Host {
    /// Every request's answer, the requests on their way side by side.
    fn fetch_all(&self, requests: Vec<http::Request>) -> Vec<Result<http::Response, Error>>;
    fn get(&self, key: &str) -> Option<Vec<u8>>;
    fn set(&self, key: &str, value: &[u8]);
    fn remove(&self, key: &str);
    fn log(&self, line: &str);
}

/// The host's imports.
#[derive(Default)]
pub(crate) struct Nori;

impl Host for Nori {
    fn fetch_all(&self, requests: Vec<http::Request>) -> Vec<Result<http::Response, Error>> {
        fetching::run(All(requests.into_iter().map(|r| Some(Err(fetching::send(r)))).collect()))
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

type Answer = Result<http::Response, Error>;

/// Requests polled together, so they are started together: each still asked (`Err`) or answered (`Ok`).
struct All(Vec<Option<Result<Answer, fetching::Fetch>>>);

impl Future for All {
    type Output = Vec<Answer>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context) -> Poll<Vec<Answer>> {
        let mut waiting = false;
        for slot in self.0.iter_mut() {
            if let Some(Err(fetch)) = slot {
                match Pin::new(fetch).poll(cx) {
                    Poll::Ready(answer) => *slot = Some(Ok(answer)),
                    Poll::Pending => waiting = true,
                }
            }
        }
        if waiting {
            return Poll::Pending;
        }
        Poll::Ready(self.0.iter_mut().map(|s| s.take().and_then(Result::ok).expect("answered")).collect())
    }
}
