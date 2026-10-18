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
        pub path: String,
        pub headers: http::HeaderMap,
        pub body: serde_json::Value,
    }

    #[derive(Default)]
    pub(crate) struct Replay {
        answers: Mutex<VecDeque<(&'static str, u16, String)>>,
        pub asked: Mutex<Vec<Asked>>,
    }

    impl Replay {
        /// The next answer: to a path starting with `path`.
        pub(crate) fn then(self, path: &'static str, status: u16, body: &str) -> Self {
            self.answers
                .lock()
                .unwrap()
                .push_back((path, status, body.to_owned()));
            self
        }

        pub(crate) fn left(&self) -> usize {
            self.answers.lock().unwrap().len()
        }

        /// Which server the `n`-th request went to.
        pub(crate) fn host(&self, n: usize) -> String {
            self.asked.lock().unwrap()[n].host.clone()
        }

        pub(crate) fn asked(&self, n: usize) -> (String, http::HeaderMap, serde_json::Value) {
            let asked = self.asked.lock().unwrap();
            let a = &asked[n];
            (a.path.clone(), a.headers.clone(), a.body.clone())
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
                let (expected, status, answer) = self
                    .answers
                    .lock()
                    .unwrap()
                    .pop_front()
                    .unwrap_or_else(|| panic!("nothing more to answer to {path}"));
                assert!(
                    path.starts_with(expected),
                    "asked {path}, expected {expected}"
                );
                self.asked.lock().unwrap().push(Asked {
                    host: host.to_owned(),
                    path,
                    headers: parts.headers,
                    body: serde_json::from_slice(&body).unwrap_or(serde_json::Value::Null),
                });
                Ok((
                    StatusCode::from_u16(status)?,
                    Bytes::from(answer.into_bytes()),
                ))
            })
        }
    }
}
