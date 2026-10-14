//! R02 discovery experiments. No selector is verified on stock Windows yet.
//! Geometry validation is portable; HWNDs and COM objects never leave the probe.
pub mod candidate;
pub mod gate;
pub mod geometry;
#[cfg(windows)]
pub mod helper;
#[cfg(windows)]
pub mod probe;
pub mod protocol;
#[cfg(windows)]
pub mod uia;
pub mod worker;

use clock_core::ShellGeneration;

/// One active request plus one coalesced pending request. The owner must call
/// `finish` only after helper exit is observed; timeout alone does not free it.
#[derive(Debug, Default)]
pub struct ProbeQueue {
    active: Option<ShellGeneration>,
    pending: Option<ShellGeneration>,
    current: Option<ShellGeneration>,
}

impl ProbeQueue {
    pub fn request(&mut self, generation: ShellGeneration) {
        self.current = Some(generation);
        self.pending = Some(generation);
    }

    pub fn start(&mut self) -> Option<ShellGeneration> {
        if self.active.is_some() {
            return None;
        }
        self.active = self.pending.take();
        self.active
    }

    /// Returns whether the completed generation may publish. A superseding
    /// request invalidates the result even if the helper finished successfully.
    pub fn finish(&mut self, generation: ShellGeneration) -> bool {
        if self.active != Some(generation) {
            return false;
        }
        self.active = None;
        self.current == Some(generation)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn coalesces_without_starting_a_second_worker_and_rejects_stale_results() {
        let mut queue = ProbeQueue::default();
        queue.request(ShellGeneration(1));
        assert_eq!(queue.start(), Some(ShellGeneration(1)));
        queue.request(ShellGeneration(2));
        queue.request(ShellGeneration(3));
        assert_eq!(queue.start(), None);
        assert!(!queue.finish(ShellGeneration(2)));
        assert_eq!(queue.start(), None);
        assert!(!queue.finish(ShellGeneration(1)));
        assert_eq!(queue.start(), Some(ShellGeneration(3)));
        assert!(queue.finish(ShellGeneration(3)));
        assert_eq!(queue.start(), None);
    }
}
