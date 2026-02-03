// SPDX-License-Identifier: MPL-2.0

//! Frame scheduling infrastructure for animated wallpapers.
//!
//! This module provides timing coordination for animated wallpaper sources,
//! ensuring smooth frame delivery synchronized with Wayland frame callbacks.

use std::{
    cmp::Ordering,
    collections::BinaryHeap,
    time::{Duration, Instant},
};

/// A scheduled frame for a specific output.
#[derive(Debug, Clone)]
struct ScheduledFrame {
    /// Name of the output this frame is for
    output: String,
    /// When this frame should be rendered
    deadline: Instant,
}

impl ScheduledFrame {
    fn new(output: impl Into<String>, deadline: Instant) -> Self {
        Self {
            output: output.into(),
            deadline,
        }
    }
}

// Implement ordering for min-heap (earliest deadline first)
impl PartialEq for ScheduledFrame {
    fn eq(&self, other: &Self) -> bool {
        self.deadline == other.deadline
    }
}

impl Eq for ScheduledFrame {}

impl PartialOrd for ScheduledFrame {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for ScheduledFrame {
    fn cmp(&self, other: &Self) -> Ordering {
        // Reverse ordering for min-heap (BinaryHeap is max-heap by default)
        other.deadline.cmp(&self.deadline)
    }
}

/// Frame scheduler for managing animated wallpaper timing.
///
/// Uses a priority queue (min-heap) to track frame deadlines across
/// multiple outputs, allowing efficient retrieval of the next frame
/// that needs to be rendered.
#[derive(Debug, Default)]
pub struct FrameScheduler {
    /// Priority queue of scheduled frames (min-heap by deadline)
    queue: BinaryHeap<ScheduledFrame>,
}

impl FrameScheduler {
    /// Create a new frame scheduler.
    pub fn new() -> Self {
        Self::default()
    }

    /// Schedule a frame for the given output after the specified duration.
    ///
    /// # Arguments
    /// * `output` - Name of the output display
    /// * `duration` - Time until the frame should be rendered
    pub fn schedule(&mut self, output: impl Into<String>, duration: Duration) {
        let deadline = Instant::now() + duration;
        let frame = ScheduledFrame::new(output, deadline);

        tracing::trace!(
            output = %frame.output,
            deadline_ms = duration.as_millis(),
            "scheduled frame"
        );

        self.queue.push(frame);
    }

    /// Schedule a frame for the given output at a specific deadline.
    ///
    /// # Arguments
    /// * `output` - Name of the output display
    /// * `deadline` - Absolute time when the frame should be rendered
    pub fn schedule_at(&mut self, output: impl Into<String>, deadline: Instant) {
        let frame = ScheduledFrame::new(output, deadline);

        tracing::trace!(
            output = %frame.output,
            "scheduled frame at absolute time"
        );

        self.queue.push(frame);
    }

    /// Returns the duration until the next scheduled frame, if any.
    ///
    /// Returns `None` if no frames are scheduled.
    /// Returns `Duration::ZERO` if a frame is already overdue.
    pub fn next_deadline(&self) -> Option<Duration> {
        self.queue.peek().map(|frame| {
            let now = Instant::now();
            if frame.deadline > now {
                frame.deadline - now
            } else {
                Duration::ZERO
            }
        })
    }

    /// Returns the instant of the next scheduled frame, if any.
    pub fn next_deadline_instant(&self) -> Option<Instant> {
        self.queue.peek().map(|frame| frame.deadline)
    }

    /// Pop all frames that are ready to render (deadline has passed).
    ///
    /// Returns a vector of output names for frames that should be rendered now.
    pub fn pop_ready(&mut self) -> Vec<String> {
        let mut ready = Vec::new();
        while let Some(output) = self.pop_next_ready() {
            ready.push(output);
        }
        ready
    }

    /// Pop the next ready frame, if any.
    ///
    /// Returns `Some(output_name)` if a frame is ready, `None` otherwise.
    pub fn pop_next_ready(&mut self) -> Option<String> {
        let now = Instant::now();

        if let Some(frame) = self.queue.peek() {
            if frame.deadline <= now {
                return self.queue.pop().map(|f| {
                    tracing::trace!(output = %f.output, "frame ready");
                    f.output
                });
            }
        }

        None
    }

    /// Returns the number of scheduled frames.
    pub fn len(&self) -> usize {
        self.queue.len()
    }

    /// Returns true if no frames are scheduled.
    pub fn is_empty(&self) -> bool {
        self.queue.is_empty()
    }

    /// Clear all scheduled frames.
    pub fn clear(&mut self) {
        self.queue.clear();
        tracing::trace!("cleared all scheduled frames");
    }

    /// Remove all scheduled frames for a specific output.
    ///
    /// # Performance
    /// This operation is O(n log n) due to heap reconstruction after filtering.
    /// For frequent removals, consider tracking frames per output separately.
    pub fn remove_output(&mut self, output: &str) {
        let frames: Vec<_> = self.queue.drain().filter(|f| f.output != output).collect();
        self.queue = frames.into_iter().collect();
        tracing::trace!(output, "removed frames for output");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread::sleep;

    #[test]
    fn test_new_scheduler_is_empty() {
        let scheduler = FrameScheduler::new();
        assert!(scheduler.is_empty());
        assert_eq!(scheduler.len(), 0);
        assert!(scheduler.next_deadline().is_none());
    }

    #[test]
    fn test_schedule_and_deadline() {
        let mut scheduler = FrameScheduler::new();

        scheduler.schedule("output1", Duration::from_millis(100));

        assert!(!scheduler.is_empty());
        assert_eq!(scheduler.len(), 1);

        let deadline = scheduler.next_deadline();
        assert!(deadline.is_some());
        assert!(deadline.unwrap() <= Duration::from_millis(100));
    }

    #[test]
    fn test_priority_ordering() {
        let mut scheduler = FrameScheduler::new();

        // Schedule in reverse order
        scheduler.schedule("output3", Duration::from_millis(300));
        scheduler.schedule("output1", Duration::from_millis(100));
        scheduler.schedule("output2", Duration::from_millis(200));

        assert_eq!(scheduler.len(), 3);

        // Earliest deadline should be first
        let deadline = scheduler.next_deadline().unwrap();
        assert!(deadline <= Duration::from_millis(100));
    }

    #[test]
    fn test_pop_ready() {
        let mut scheduler = FrameScheduler::new();

        // Schedule frames with very short deadline
        scheduler.schedule("output1", Duration::from_millis(1));
        scheduler.schedule("output2", Duration::from_millis(1));

        // Wait for them to be ready
        sleep(Duration::from_millis(10));

        let ready = scheduler.pop_ready();
        assert_eq!(ready.len(), 2);
        assert!(scheduler.is_empty());
    }

    #[test]
    fn test_pop_ready_respects_deadline() {
        let mut scheduler = FrameScheduler::new();

        scheduler.schedule("ready", Duration::from_millis(1));
        scheduler.schedule("not_ready", Duration::from_secs(10));

        sleep(Duration::from_millis(10));

        let ready = scheduler.pop_ready();
        assert_eq!(ready.len(), 1);
        assert_eq!(ready[0], "ready");
        assert_eq!(scheduler.len(), 1); // "not_ready" still scheduled
    }

    #[test]
    fn test_clear() {
        let mut scheduler = FrameScheduler::new();

        scheduler.schedule("output1", Duration::from_millis(100));
        scheduler.schedule("output2", Duration::from_millis(200));

        assert_eq!(scheduler.len(), 2);

        scheduler.clear();

        assert!(scheduler.is_empty());
    }

    #[test]
    fn test_remove_output() {
        let mut scheduler = FrameScheduler::new();

        scheduler.schedule("keep", Duration::from_millis(100));
        scheduler.schedule("remove", Duration::from_millis(100));
        scheduler.schedule("keep", Duration::from_millis(200));

        assert_eq!(scheduler.len(), 3);

        scheduler.remove_output("remove");

        assert_eq!(scheduler.len(), 2);
    }
}
