//! A doorbell's or camera's last image, on demand. Ring keeps one snapshot
//! per device on its servers; a fresh one is asked (`PUT snapshots/update`),
//! takes a few seconds (`POST snapshots/timestamps` says when the latest
//! was taken), then is read (`GET snapshots/image/<id>`). A device on
//! battery wakes up for it, and gives none while it records: so one is
//! asked seldom, only when the dashboard shows it, one at a time, and the
//! image kept is given back with its own time, never as a new one, when
//! Ring has nothing newer.

use std::sync::Arc;
use std::time::Duration;

use anyhow::bail;
use moli_core::DeviceId;
use moli_runtime::BoxFuture;
use moli_runtime::media::{Image, SnapshotSource};
use tokio::sync::{Mutex, mpsc};
use tokio::time::Instant;

use crate::api::{Client, Failure, Vault};
use crate::model::{self, RingDevice};

/// On battery: no new image asked within this of the last one.
const BATTERY_GAP: Duration = Duration::from_secs(60);
/// On mains: the same, shorter.
const MAINS_GAP: Duration = Duration::from_secs(15);
/// A fresh snapshot takes a few seconds: its time is read this often…
const WAIT: Duration = Duration::from_secs(2);
/// …this many times at most (the dashboard waits 20 s in all).
const TRIES: u32 = 5;

/// The account's client, shared with the driver's loop.
pub(crate) type Shared = Arc<Mutex<Client>>;

/// A new image's time, told to the driver's loop (it shows `image_at`).
pub(crate) type Taken = mpsc::UnboundedSender<(DeviceId, i64)>;

/// One device's images.
pub(crate) struct Images {
    client: Shared,
    vault: Arc<dyn Vault>,
    /// Ring's number for the device.
    id: u64,
    name: String,
    battery: bool,
    device: DeviceId,
    taken: Taken,
    cache: Mutex<Cache>,
}

#[derive(Default)]
struct Cache {
    /// The last image read, and when Ring says it was taken.
    image: Option<(Option<i64>, Image)>,
    /// When one was last asked of Ring.
    asked: Option<Instant>,
}

impl std::fmt::Debug for Images {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Images")
            .field("device", &self.device)
            .field("battery", &self.battery)
            .finish_non_exhaustive()
    }
}

impl Images {
    pub(crate) fn new(
        client: Shared,
        vault: Arc<dyn Vault>,
        d: &RingDevice,
        device: DeviceId,
        taken: Taken,
    ) -> Self {
        Self {
            client,
            vault,
            id: d.id,
            name: d.name.clone(),
            battery: d.on_battery(),
            device,
            taken,
            cache: Mutex::new(Cache::default()),
        }
    }

    async fn take(&self) -> anyhow::Result<Image> {
        let mut cache = self.cache.lock().await;
        let gap = if self.battery { BATTERY_GAP } else { MAINS_GAP };
        if let (Some(asked), Some((_, image))) = (cache.asked, &cache.image)
            && asked.elapsed() < gap
        {
            return Ok(image.clone());
        }
        cache.asked = Some(Instant::now());
        let known = cache.image.as_ref().and_then(|(at, _)| *at);
        let ids = model::doorbot_ids(self.id);
        // A fresh one, asked once: Ring answers in a few seconds, or not
        // at all (recording, asleep); then the last one it has will do.
        let mut refresh = ids.clone();
        refresh["refresh"] = serde_json::Value::Bool(true);
        match self
            .client
            .lock()
            .await
            .put(&*self.vault, "/clients_api/snapshots/update", &refresh)
            .await
        {
            Ok(_) => {}
            Err(Failure::Other(e)) => {
                tracing::debug!(device = %self.name, error = %format!("{e:#}"), "ring snapshot update");
            }
            Err(e) => bail!("{e}"),
        }
        let mut latest = None;
        for _ in 0..TRIES {
            tokio::time::sleep(WAIT).await;
            let answer = self
                .client
                .lock()
                .await
                .post(&*self.vault, "/clients_api/snapshots/timestamps", &ids)
                .await;
            match answer {
                Ok(a) => latest = model::snapshot_taken(&a, self.id),
                Err(Failure::Other(e)) => {
                    tracing::debug!(device = %self.name, error = %format!("{e:#}"), "ring snapshot time");
                }
                Err(e) => bail!("{e}"),
            }
            if latest.is_some() && latest > known {
                break;
            }
        }
        // Nothing newer than the image kept: it stays, with its own time.
        if let Some((Some(had), image)) = &cache.image
            && latest.is_some_and(|t| t <= *had)
        {
            return Ok(image.clone());
        }
        let bytes = self
            .client
            .lock()
            .await
            .fetch(&*self.vault, &self.image_path(), "image/jpeg")
            .await
            .map_err(|e| anyhow::anyhow!("{e}"))?;
        match bytes {
            Some(b) if b.starts_with(&[0xFF, 0xD8]) => {
                let image = Image {
                    content_type: "image/jpeg".into(),
                    bytes: b.to_vec(),
                    taken: latest,
                };
                cache.image = Some((latest, image.clone()));
                if let Some(at) = latest {
                    let _ = self.taken.send((self.device.clone(), at));
                }
                Ok(image)
            }
            // Ring has none right now (recording, asleep): the last one.
            _ => match &cache.image {
                Some((_, image)) => Ok(image.clone()),
                None => bail!(moli_i18n::tr!("pilotes.ring.aucune_image")),
            },
        }
    }

    fn image_path(&self) -> String {
        format!("/clients_api/snapshots/image/{}", self.id)
    }
}

impl SnapshotSource for Images {
    fn snapshot(&self) -> BoxFuture<'_, anyhow::Result<Image>> {
        Box::pin(self.take())
    }
}
