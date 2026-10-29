//! Independent bounded queues. Control admission never bypasses remote locks.
use std::sync::Arc;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

pub(super) async fn admit(
    slots: Arc<Semaphore>,
    cancelled: impl std::future::Future<Output = ()>,
) -> Result<Option<OwnedSemaphorePermit>, tokio::sync::AcquireError> {
    tokio::select! {biased;_=cancelled=>Ok(None),result=slots.acquire_owned()=>result.map(Some)}
}
struct Lane {
    slots: Arc<Semaphore>,
    queued: Arc<Semaphore>,
    bytes: Arc<Semaphore>,
}
impl Lane {
    fn new(slots: usize, queue: usize, bytes: usize) -> Self {
        Self {
            slots: Arc::new(Semaphore::new(slots)),
            queued: Arc::new(Semaphore::new(queue)),
            bytes: Arc::new(Semaphore::new(bytes)),
        }
    }
    async fn acquire(
        &self,
        bytes: usize,
        cancelled: impl std::future::Future<Output = ()>,
    ) -> Result<Option<OwnedSemaphorePermit>, ()> {
        tokio::pin!(cancelled);
        if tokio::select! {biased;_=&mut cancelled=>true,_=std::future::ready(())=>false} {
            return Ok(None);
        }
        if let Ok(permit) = self.slots.clone().try_acquire_owned() {
            return Ok(Some(permit));
        }
        let _count = self.queued.clone().try_acquire_owned().map_err(|_| ())?;
        let bytes = u32::try_from(bytes.max(1)).map_err(|_| ())?;
        let _bytes = self
            .bytes
            .clone()
            .try_acquire_many_owned(bytes)
            .map_err(|_| ())?;
        admit(self.slots.clone(), &mut cancelled)
            .await
            .map_err(|_| ())
    }
}
pub(super) struct Admission {
    ordinary: Lane,
    control: Lane,
    local: Lane,
}
impl Default for Admission {
    fn default() -> Self {
        Self {
            ordinary: Lane::new(16, 32, 32 * 1024 * 1024),
            control: Lane::new(2, 8, 4 * 1024 * 1024),
            local: Lane::new(2, 8, 4 * 1024 * 1024),
        }
    }
}
impl Admission {
    pub async fn acquire(
        &self,
        operation: &str,
        bytes: usize,
        cancelled: impl std::future::Future<Output = ()>,
    ) -> Result<Option<OwnedSemaphorePermit>, ()> {
        let lane = match operation {
            "version" | "hosts" => &self.local,
            "run.signal" | "session.recover" | "session.close" => &self.control,
            _ => &self.ordinary,
        };
        lane.acquire(bytes, cancelled).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn full_ordinary_lane_leaves_control_and_local_capacity() {
        let a = Admission::default();
        let mut held = Vec::new();
        for _ in 0..16 {
            held.push(
                a.acquire("run.poll", 10, std::future::pending())
                    .await
                    .ok()
                    .flatten()
                    .unwrap_or_else(|| panic!("permit")),
            );
        }
        assert!(
            a.acquire("run.signal", 10, std::future::pending())
                .await
                .ok()
                .flatten()
                .is_some()
        );
        assert!(
            a.acquire("version", 10, std::future::pending())
                .await
                .ok()
                .flatten()
                .is_some()
        );
        assert_eq!(a.ordinary.slots.available_permits(), 0);
    }
    #[tokio::test]
    async fn queue_count_and_bytes_refuse_without_waiting_and_release_on_cancel() {
        let lane = Lane::new(1, 1, 8);
        let held = lane
            .slots
            .clone()
            .try_acquire_owned()
            .unwrap_or_else(|e| panic!("hold: {e}"));
        assert!(lane.acquire(9, std::future::pending()).await.is_err());
        let count = lane
            .queued
            .clone()
            .try_acquire_owned()
            .unwrap_or_else(|e| panic!("queue: {e}"));
        assert!(lane.acquire(1, std::future::pending()).await.is_err());
        drop(count);
        assert!(
            lane.acquire(8, std::future::ready(()))
                .await
                .ok()
                .flatten()
                .is_none()
        );
        assert_eq!(lane.queued.available_permits(), 1);
        assert_eq!(lane.bytes.available_permits(), 8);
        drop(held);
        assert!(
            lane.acquire(8, std::future::pending())
                .await
                .ok()
                .flatten()
                .is_some()
        );
    }
}
