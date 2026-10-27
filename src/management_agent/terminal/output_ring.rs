use super::TerminalError;
use std::collections::VecDeque;

/// 每个输出块在数据之外的固定内存开销估计（结构体与一次堆分配），计入容量，
/// 防止大量 1 字节输出让实际内存远超限额。
pub const CHUNK_OVERHEAD: usize = 64;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OutputChunk {
    pub seq: u64,
    pub data: Vec<u8>,
}

/// 按字节限额保留最近的输出块；序号从 1 起单调递增，丢弃旧块不影响序号连续。
#[derive(Debug)]
pub struct OutputRing {
    capacity: usize,
    chunks: VecDeque<OutputChunk>,
    retained_bytes: usize,
    accounted_bytes: usize,
    latest_seq: u64,
}

impl OutputRing {
    pub fn new(capacity: usize) -> Self {
        Self {
            capacity: capacity.max(CHUNK_OVERHEAD + 1),
            chunks: VecDeque::new(),
            retained_bytes: 0,
            accounted_bytes: 0,
            latest_seq: 0,
        }
    }

    pub fn push(&mut self, data: &[u8]) -> Option<u64> {
        if data.is_empty() {
            return None;
        }
        // PTY 单次读取远小于限额；超限的块只留尾部，序号照常分配。
        let room = self.capacity - CHUNK_OVERHEAD;
        let data = &data[data.len().saturating_sub(room)..];
        self.latest_seq += 1;
        self.retained_bytes += data.len();
        self.accounted_bytes += data.len() + CHUNK_OVERHEAD;
        self.chunks.push_back(OutputChunk {
            seq: self.latest_seq,
            data: data.to_vec(),
        });
        while self.accounted_bytes > self.capacity {
            match self.chunks.pop_front() {
                Some(dropped) => {
                    self.retained_bytes -= dropped.data.len();
                    self.accounted_bytes -= dropped.data.len() + CHUNK_OVERHEAD;
                }
                None => break,
            }
        }
        Some(self.latest_seq)
    }

    /// 返回 `last_seq` 之后的块，累计数据不超过 `max_bytes`；至少返回一块，
    /// 调用方用最后一块的序号继续读取。
    pub fn since(
        &self,
        last_seq: u64,
        max_bytes: usize,
    ) -> Result<Vec<OutputChunk>, TerminalError> {
        if last_seq > self.latest_seq {
            return Err(TerminalError::SeqAhead {
                requested: last_seq,
                latest: self.latest_seq,
            });
        }
        let Some(oldest) = self.oldest_seq() else {
            return Ok(Vec::new());
        };
        if last_seq + 1 < oldest {
            return Err(TerminalError::GapTooLarge {
                requested: last_seq,
                oldest,
            });
        }
        let start = (last_seq + 1 - oldest) as usize;
        let mut total = 0usize;
        let mut result = Vec::new();
        for chunk in self.chunks.iter().skip(start) {
            if !result.is_empty() && total + chunk.data.len() > max_bytes {
                break;
            }
            total += chunk.data.len();
            result.push(chunk.clone());
        }
        Ok(result)
    }

    pub fn latest_seq(&self) -> u64 {
        self.latest_seq
    }

    pub fn oldest_seq(&self) -> Option<u64> {
        self.chunks.front().map(|chunk| chunk.seq)
    }

    pub fn retained_bytes(&self) -> usize {
        self.retained_bytes
    }

    pub fn chunk_count(&self) -> usize {
        self.chunks.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seqs(chunks: &[OutputChunk]) -> Vec<u64> {
        chunks.iter().map(|chunk| chunk.seq).collect()
    }

    fn all(ring: &OutputRing, last_seq: u64) -> Result<Vec<OutputChunk>, TerminalError> {
        ring.since(last_seq, usize::MAX)
    }

    /// 容量正好放得下 `count` 个 `size` 字节的块。
    fn room(count: usize, size: usize) -> usize {
        count * (size + CHUNK_OVERHEAD)
    }

    #[test]
    fn sequences_start_at_one_and_increase() {
        let mut ring = OutputRing::new(room(4, 4));
        assert_eq!(ring.push(b"a"), Some(1));
        assert_eq!(ring.push(b"bc"), Some(2));
        assert_eq!(ring.latest_seq(), 2);
        assert_eq!(ring.retained_bytes(), 3);
    }

    #[test]
    fn empty_push_consumes_no_sequence() {
        let mut ring = OutputRing::new(room(4, 4));
        assert_eq!(ring.push(b""), None);
        assert_eq!(ring.push(b"x"), Some(1));
    }

    #[test]
    fn since_returns_only_newer_chunks_byte_exact() {
        let mut ring = OutputRing::new(room(4, 4));
        ring.push(b"one");
        ring.push(&[0xe4, 0xb8]);
        ring.push(&[0xad, 0x1b, b'[']);
        let chunks = all(&ring, 1).unwrap();
        assert_eq!(seqs(&chunks), vec![2, 3]);
        assert_eq!(chunks[0].data, vec![0xe4, 0xb8]);
        assert_eq!(chunks[1].data, vec![0xad, 0x1b, b'[']);
        assert_eq!(all(&ring, 3).unwrap(), Vec::new());
    }

    #[test]
    fn overflow_drops_oldest_but_keeps_sequence_continuous() {
        let mut ring = OutputRing::new(room(2, 4));
        for _ in 0..5 {
            ring.push(b"abcd");
        }
        assert_eq!(ring.latest_seq(), 5);
        assert_eq!(ring.oldest_seq(), Some(4));
        assert_eq!(seqs(&all(&ring, 3).unwrap()), vec![4, 5]);
        assert_eq!(seqs(&all(&ring, 4).unwrap()), vec![5]);
    }

    #[test]
    fn tiny_chunks_cannot_exceed_memory_budget() {
        let capacity = room(10, 1);
        let mut ring = OutputRing::new(capacity);
        for _ in 0..10_000 {
            ring.push(b"x");
        }
        assert_eq!(ring.latest_seq(), 10_000);
        assert!(ring.chunk_count() <= 10);
        assert!(ring.chunk_count() * (1 + CHUNK_OVERHEAD) <= capacity);
    }

    #[test]
    fn resume_point_already_discarded_is_a_gap() {
        let mut ring = OutputRing::new(room(2, 4));
        for _ in 0..5 {
            ring.push(b"abcd");
        }
        assert_eq!(
            all(&ring, 2),
            Err(TerminalError::GapTooLarge {
                requested: 2,
                oldest: 4
            })
        );
        assert_eq!(
            all(&ring, 0),
            Err(TerminalError::GapTooLarge {
                requested: 0,
                oldest: 4
            })
        );
    }

    #[test]
    fn resume_point_ahead_of_latest_is_rejected() {
        let mut ring = OutputRing::new(room(2, 4));
        ring.push(b"a");
        assert_eq!(
            all(&ring, 7),
            Err(TerminalError::SeqAhead {
                requested: 7,
                latest: 1
            })
        );
    }

    #[test]
    fn from_start_without_loss_returns_everything() {
        let mut ring = OutputRing::new(room(2, 4));
        assert_eq!(all(&ring, 0).unwrap(), Vec::new());
        ring.push(b"ab");
        assert_eq!(seqs(&all(&ring, 0).unwrap()), vec![1]);
    }

    #[test]
    fn oversized_chunk_keeps_its_tail() {
        let mut ring = OutputRing::new(room(1, 4));
        ring.push(b"xy");
        assert_eq!(ring.push(b"0123456789"), Some(2));
        let chunks = all(&ring, 1).unwrap();
        assert_eq!(
            chunks,
            vec![OutputChunk {
                seq: 2,
                data: b"6789".to_vec()
            }]
        );
        assert_eq!(ring.retained_bytes(), 4);
    }

    #[test]
    fn reads_stop_at_byte_limit_but_return_at_least_one_chunk() {
        let mut ring = OutputRing::new(room(8, 4));
        for _ in 0..3 {
            ring.push(b"abcd");
        }
        assert_eq!(seqs(&ring.since(0, 8).unwrap()), vec![1, 2]);
        assert_eq!(seqs(&ring.since(0, 11).unwrap()), vec![1, 2]);
        assert_eq!(seqs(&ring.since(0, 1).unwrap()), vec![1]);
        assert_eq!(seqs(&ring.since(2, 8).unwrap()), vec![3]);
    }
}
