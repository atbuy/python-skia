use std::collections::VecDeque;
use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Segment {
    pub path: PathBuf,
    pub start_ms: u64,
    pub end_ms: u64,
}

impl Segment {
    pub fn new(path: impl Into<PathBuf>, start_ms: u64, end_ms: u64) -> Self {
        assert!(end_ms > start_ms, "segment end must be after start");
        Self {
            path: path.into(),
            start_ms,
            end_ms,
        }
    }
}

#[derive(Debug, Clone)]
pub struct SegmentRing {
    retention_ms: u64,
    segments: VecDeque<Segment>,
}

impl SegmentRing {
    pub fn new(clip_seconds: u64, slack_seconds: u64) -> Self {
        Self {
            retention_ms: (clip_seconds + slack_seconds) * 1000,
            segments: VecDeque::new(),
        }
    }

    pub fn push(&mut self, segment: Segment) -> Vec<Segment> {
        self.segments.push_back(segment);
        self.prune()
    }

    pub fn select_last(&self, duration_seconds: u64) -> Vec<Segment> {
        let Some(latest) = self.segments.back() else {
            return Vec::new();
        };

        let cutoff_ms = latest.end_ms.saturating_sub(duration_seconds * 1000);
        self.segments
            .iter()
            .filter(|segment| segment.end_ms > cutoff_ms)
            .cloned()
            .collect()
    }

    pub fn len(&self) -> usize {
        self.segments.len()
    }

    pub fn is_empty(&self) -> bool {
        self.segments.is_empty()
    }

    fn prune(&mut self) -> Vec<Segment> {
        let Some(latest) = self.segments.back() else {
            return Vec::new();
        };

        let cutoff_ms = latest.end_ms.saturating_sub(self.retention_ms);
        let mut pruned = Vec::new();

        while self
            .segments
            .front()
            .is_some_and(|segment| segment.end_ms <= cutoff_ms)
        {
            if let Some(segment) = self.segments.pop_front() {
                pruned.push(segment);
            }
        }

        pruned
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn push_keeps_recent_segments_inside_retention() {
        let mut ring = SegmentRing::new(4, 2);

        for index in 0..4 {
            ring.push(Segment::new(
                format!("segment-{index}.mkv"),
                index * 2000,
                (index + 1) * 2000,
            ));
        }

        assert_eq!(ring.len(), 3);
    }

    #[test]
    fn push_returns_pruned_segments() {
        let mut ring = SegmentRing::new(2, 0);

        ring.push(Segment::new("segment-0.mkv", 0, 1000));
        ring.push(Segment::new("segment-1.mkv", 1000, 2000));
        let pruned = ring.push(Segment::new("segment-2.mkv", 2000, 3000));

        assert_eq!(pruned, vec![Segment::new("segment-0.mkv", 0, 1000)]);
        assert_eq!(ring.len(), 2);
    }

    #[test]
    fn select_last_returns_overlapping_segments() {
        let mut ring = SegmentRing::new(30, 6);

        ring.push(Segment::new("segment-0.mkv", 0, 2000));
        ring.push(Segment::new("segment-1.mkv", 2000, 4000));
        ring.push(Segment::new("segment-2.mkv", 4000, 6000));

        let selected = ring.select_last(3);

        assert_eq!(
            selected,
            vec![
                Segment::new("segment-1.mkv", 2000, 4000),
                Segment::new("segment-2.mkv", 4000, 6000),
            ]
        );
    }

    #[test]
    fn select_last_empty_ring_returns_empty() {
        let ring = SegmentRing::new(30, 6);

        assert!(ring.select_last(30).is_empty());
    }
}
