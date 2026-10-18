//! What a device can do beyond its points: a document to read (a heating
//! schedule, a week of slots) and actions on it, too structured for a
//! point's value. A driver registers a [`Service`] for one of its devices
//! with [`crate::DriverCtx::provide_service`]; a surface calls it through
//! [`crate::Hub::service_read`] and [`crate::Hub::service_write`]. Nothing is
//! queued: a question nobody asked is never put to the device.
//!
//! An action is a person's gesture (the dashboard, with the code), never an
//! agent's nor an automation's: the hub journals each one.

use serde_json::Value as Json;

use crate::BoxFuture;

/// Why a service gave nothing.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ServiceError {
    #[error("unknown device")]
    UnknownDevice,
    #[error("this device has no such service")]
    NoService,
    #[error("unknown action: {0}")]
    UnknownAction(String),
    /// What was sent is wrong (a value, a reference…): said to the person.
    #[error("{0}")]
    Invalid(String),
    /// The device (or its cloud) said no.
    #[error("{0}")]
    Refused(String),
    /// Not reachable, no answer in time.
    #[error("{0}")]
    Unavailable(String),
}

/// A device's own document and the actions on it.
pub trait Service: std::fmt::Debug + Send + Sync + 'static {
    /// Answers `what` (a name the device's doc gives) without changing
    /// anything.
    fn read(&self, what: &str) -> BoxFuture<'_, Result<Json, ServiceError>>;
    /// Does `action` with `input`, and answers with what the device now
    /// says (so that the person sees the result at once).
    fn write(&self, action: &str, input: Json) -> BoxFuture<'_, Result<Json, ServiceError>>;
}
