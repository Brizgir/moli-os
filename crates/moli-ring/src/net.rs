//! How Moli reaches Ring's servers: HTTPS on the Internet, verified
//! against the public authorities; in the tests, answers captured once and
//! played back.

use std::time::Duration;

use http::{Request, StatusCode};
use http_body_util::Full;
use moli_net::Body as Bytes;
use moli_runtime::BoxFuture;

const LIMIT: Duration = Duration::from_secs(20);
const MAX_BODY: usize = 4 * 1024 * 1024;

pub(crate) trait Transport: Send + Sync {
    fn send<'a>(
        &'a self,
        host: &'a str,
        request: Request<Full<Bytes>>,
    ) -> BoxFuture<'a, anyhow::Result<(StatusCode, Bytes)>>;
}

/// The real thing.
pub(crate) struct Web;

impl Transport for Web {
    fn send<'a>(
        &'a self,
        host: &'a str,
        request: Request<Full<Bytes>>,
    ) -> BoxFuture<'a, anyhow::Result<(StatusCode, Bytes)>> {
        Box::pin(moli_net::web(host, 443, true, request, LIMIT, MAX_BODY))
    }
}

#[cfg(test)]
pub(crate) mod replay {
    //! Answers given in order, each for the path it expects; what was asked
    //! is kept for the test to read.

    use std::collections::VecDeque;
    use std::sync::Mutex;

    use http_body_util::BodyExt as _;

    use super::*;

    /// One request as Ring would have received it.
    #[derive(Debug)]
    pub(crate) struct Asked {
        pub host: String,
        pub method: String,
        pub path: String,
        pub headers: http::HeaderMap,
        pub body: serde_json::Value,
        /// As sent (a protobuf, a form).
        pub raw: Vec<u8>,
    }

    #[derive(Default)]
    pub(crate) struct Replay {
        answers: Mutex<VecDeque<(&'static str, u16, Vec<u8>)>>,
        /// Answer the first queued entry whose path matches, not the next
        /// one: for a driver that reads its devices in no fixed order.
        by_path: bool,
        /// Answers never used up (a poll that goes on), after the queue.
        sticky: Mutex<Vec<(&'static str, u16, Vec<u8>)>>,
        pub asked: Mutex<Vec<Asked>>,
    }

    impl Replay {
        /// The next answer: to a path starting with `path` (`"POST /x"`:
        /// that method only).
        pub(crate) fn then(self, path: &'static str, status: u16, body: &str) -> Self {
            self.then_bytes(path, status, body.as_bytes())
        }

        pub(crate) fn then_bytes(self, path: &'static str, status: u16, body: &[u8]) -> Self {
            self.push(path, status, body);
            self
        }

        /// Queues one more answer, once the replay is in use.
        pub(crate) fn push(&self, path: &'static str, status: u16, body: &[u8]) {
            self.answers
                .lock()
                .unwrap()
                .push_back((path, status, body.to_vec()));
        }

        /// Answers matched by path (first queued match), in any order.
        pub(crate) fn by_path(self) -> Self {
            Self {
                by_path: true,
                ..self
            }
        }

        /// Whether an expected `"[METHOD ]/path"` covers this request.
        fn covers(expected: &str, method: &str, path: &str) -> bool {
            match expected.split_once(' ') {
                Some((m, p)) => method == m && path.starts_with(p),
                None => path.starts_with(expected),
            }
        }

        /// An answer given every time nothing queued matches the path;
        /// replaces the one set for that path before.
        pub(crate) fn always(&self, path: &'static str, status: u16, body: &str) {
            self.always_bytes(path, status, body.as_bytes());
        }

        pub(crate) fn always_bytes(&self, path: &'static str, status: u16, body: &[u8]) {
            let mut sticky = self.sticky.lock().unwrap();
            sticky.retain(|(p, _, _)| *p != path);
            sticky.push((path, status, body.to_vec()));
        }

        pub(crate) fn left(&self) -> usize {
            self.answers.lock().unwrap().len()
        }

        /// Which server the `n`-th request went to.
        pub(crate) fn host(&self, n: usize) -> String {
            self.asked.lock().unwrap()[n].host.clone()
        }

        pub(crate) fn method(&self, n: usize) -> String {
            self.asked.lock().unwrap()[n].method.clone()
        }

        pub(crate) fn raw(&self, n: usize) -> Vec<u8> {
            self.asked.lock().unwrap()[n].raw.clone()
        }

        pub(crate) fn asked(&self, n: usize) -> (String, http::HeaderMap, serde_json::Value) {
            let asked = self.asked.lock().unwrap();
            let a = &asked[n];
            (a.path.clone(), a.headers.clone(), a.body.clone())
        }

        /// Every request made so far, as (method, path).
        pub(crate) fn calls(&self) -> Vec<(String, String)> {
            self.asked
                .lock()
                .unwrap()
                .iter()
                .map(|a| (a.method.clone(), a.path.clone()))
                .collect()
        }
    }

    impl Transport for Replay {
        fn send<'a>(
            &'a self,
            host: &'a str,
            request: Request<Full<Bytes>>,
        ) -> BoxFuture<'a, anyhow::Result<(StatusCode, Bytes)>> {
            Box::pin(async move {
                let (parts, body) = request.into_parts();
                let body = body.collect().await?.to_bytes();
                let path = parts.uri.to_string();
                let method = parts.method.to_string();
                let (expected, status, answer) = {
                    let mut answers = self.answers.lock().unwrap();
                    let at = if self.by_path {
                        answers
                            .iter()
                            .position(|(p, _, _)| Self::covers(p, &method, &path))
                    } else {
                        (!answers.is_empty()).then_some(0)
                    };
                    at.and_then(|i| answers.remove(i))
                        .or_else(|| {
                            self.sticky
                                .lock()
                                .unwrap()
                                .iter()
                                .find(|(p, _, _)| Self::covers(p, &method, &path))
                                .cloned()
                        })
                        .unwrap_or_else(|| panic!("nothing more to answer to {method} {path}"))
                };
                assert!(
                    Self::covers(expected, &method, &path),
                    "asked {method} {path}, expected {expected}"
                );
                self.asked.lock().unwrap().push(Asked {
                    host: host.to_owned(),
                    method,
                    path,
                    headers: parts.headers,
                    body: serde_json::from_slice(&body).unwrap_or(serde_json::Value::Null),
                    raw: body.to_vec(),
                });
                Ok((StatusCode::from_u16(status)?, Bytes::from(answer)))
            })
        }
    }
}
