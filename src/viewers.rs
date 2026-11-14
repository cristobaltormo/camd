use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use tokio::sync::watch;

pub const MAX_VIEWERS: usize = 4;

pub struct ViewerPool {
    pub limit: usize,
    open: Mutex<Vec<(u64, watch::Sender<bool>)>>,
    next: AtomicU64,
}

pub struct Viewer {
    id: u64,
    cancel: watch::Receiver<bool>,
    pool: Arc<ViewerPool>,
}

impl ViewerPool {
    pub fn new(limit: usize) -> Arc<ViewerPool> {
        Arc::new(ViewerPool {
            limit,
            open: Mutex::new(Vec::new()),
            next: AtomicU64::new(1),
        })
    }

    pub fn join(self: &Arc<Self>) -> Viewer {
        let id = self.next.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = watch::channel(false);
        let mut open = self.open.lock().unwrap_or_else(|e| e.into_inner());
        open.push((id, tx));
        while open.len() > self.limit {
            let (_, oldest) = open.remove(0);
            let _ = oldest.send(true);
        }
        Viewer {
            id,
            cancel: rx,
            pool: Arc::clone(self),
        }
    }

    pub fn len(&self) -> usize {
        self.open.lock().unwrap_or_else(|e| e.into_inner()).len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    fn leave(&self, id: u64) {
        self.open
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .retain(|(open, _)| *open != id);
    }
}

impl Viewer {
    pub fn is_cancelled(&self) -> bool {
        *self.cancel.borrow()
    }

    pub async fn cancelled(&mut self) {
        let _ = self.cancel.wait_for(|cancelled| *cancelled).await;
    }
}

impl Drop for Viewer {
    fn drop(&mut self) {
        self.pool.leave(self.id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_oldest_viewer_is_kicked_past_the_limit() {
        let pool = ViewerPool::new(2);
        let first = pool.join();
        let second = pool.join();
        assert_eq!(pool.len(), 2);
        let third = pool.join();
        assert!(first.is_cancelled() && !second.is_cancelled() && !third.is_cancelled());
        assert_eq!(pool.len(), 2);
        drop(second);
        assert_eq!(pool.len(), 1);
        drop(first);
        drop(third);
        assert!(pool.is_empty());
    }

    #[tokio::test]
    async fn cancellation_wakes_the_waiting_stream() {
        let pool = ViewerPool::new(1);
        let mut first = pool.join();
        let _second = pool.join();
        tokio::time::timeout(std::time::Duration::from_secs(1), first.cancelled())
            .await
            .unwrap();
    }
}
