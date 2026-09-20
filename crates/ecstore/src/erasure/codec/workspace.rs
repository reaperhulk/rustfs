// Copyright 2024 RustFS Team
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

/// A verified shard can share its source allocation until recovery mutates it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ReadShard {
    Owned(Vec<u8>),
    Shared(bytes::Bytes),
}

impl std::ops::Deref for ReadShard {
    type Target = [u8];

    fn deref(&self) -> &[u8] {
        match self {
            Self::Owned(data) => data,
            Self::Shared(data) => data,
        }
    }
}

impl AsRef<[u8]> for ReadShard {
    fn as_ref(&self) -> &[u8] {
        self
    }
}

impl AsMut<[u8]> for ReadShard {
    fn as_mut(&mut self) -> &mut [u8] {
        match self {
            Self::Owned(data) => data,
            Self::Shared(data) => {
                *self = Self::Owned(data.to_vec());
                self.as_mut()
            }
        }
    }
}

impl<'a> IntoIterator for &'a ReadShard {
    type Item = &'a u8;
    type IntoIter = std::slice::Iter<'a, u8>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

impl From<Vec<u8>> for ReadShard {
    fn from(data: Vec<u8>) -> Self {
        Self::Owned(data)
    }
}

impl FromIterator<u8> for ReadShard {
    fn from_iter<I: IntoIterator<Item = u8>>(iter: I) -> Self {
        Self::Owned(iter.into_iter().collect())
    }
}

impl crate::erasure::coding::erasure::ErasureShard for ReadShard {
    fn into_vec(self) -> Vec<u8> {
        match self {
            Self::Owned(data) => data,
            Self::Shared(data) => data.to_vec(),
        }
    }
}

impl ReadShard {
    #[cfg(test)]
    pub(crate) fn capacity(&self) -> usize {
        match self {
            Self::Owned(data) => data.capacity(),
            Self::Shared(data) => data.len(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct RustfsCodecDecodeWorkspace {
    shard_len: usize,
}

impl RustfsCodecDecodeWorkspace {
    #[inline]
    pub(crate) fn new(shard_len: usize) -> Self {
        Self { shard_len }
    }

    #[inline]
    #[allow(dead_code, reason = "workspace width asserted by decode_reader tests (backlog#1823)")]
    pub(crate) fn shard_len(&self) -> usize {
        self.shard_len
    }
}

#[derive(Debug, Default)]
pub(crate) struct ShardBufferPool {
    buffers: Vec<Option<Vec<u8>>>,
}

impl ShardBufferPool {
    #[inline]
    pub(crate) fn new(slot_count: usize) -> Self {
        let mut buffers = Vec::with_capacity(slot_count);
        buffers.resize_with(slot_count, || None);
        Self { buffers }
    }

    #[inline]
    pub(crate) fn ensure_slots(&mut self, slot_count: usize) {
        if self.buffers.len() < slot_count {
            self.buffers.resize_with(slot_count, || None);
        }
    }

    /// An **empty** buffer with room for at least `len` bytes. The caller fills it
    /// by appending (see `BitrotReader::read_appending`), so the pool never has to
    /// initialize the bytes it hands out.
    ///
    /// Zeroing here was pure waste: `read_appending` overwrites every byte it
    /// returns, and a short read is an error rather than a partial buffer, so no
    /// caller ever observes a byte the reader did not write. At 1 MiB shards the
    /// `resize(len, 0)` was ~4.8% of GET CPU (rustfs/backlog#1159) — a buffer pool
    /// exists to reuse an allocation, and memsetting it gives that saving straight
    /// back.
    #[inline]
    pub(crate) fn take(&mut self, index: usize, len: usize) -> Vec<u8> {
        self.ensure_slots(index + 1);
        let mut buf = self.buffers[index].take().unwrap_or_else(|| Vec::with_capacity(len));
        buf.clear();
        if buf.capacity() < len {
            buf.reserve_exact(len - buf.len());
        }
        buf
    }

    #[inline]
    pub(crate) fn put(&mut self, index: usize, buf: Vec<u8>) {
        self.ensure_slots(index + 1);
        self.buffers[index] = Some(buf);
    }

    #[cfg(test)]
    pub(crate) fn stored_allocation(&self, index: usize) -> Option<(*const u8, usize)> {
        self.buffers
            .get(index)
            .and_then(|buf| buf.as_ref().map(|buf| (buf.as_ptr(), buf.capacity())))
    }

    #[cfg(test)]
    fn stored_capacity(&self, index: usize) -> Option<usize> {
        self.buffers.get(index).and_then(|buf| buf.as_ref().map(Vec::capacity))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shared_read_shard_copies_only_when_mutated() {
        let source = bytes::Bytes::from(vec![11, 22, 33, 44]);
        let mut shard = ReadShard::Shared(source.slice(1..3));
        assert_eq!(shard.as_ptr(), source[1..].as_ptr());
        let retained = shard.clone();
        shard.as_mut()[0] = 99;
        assert_eq!(shard.as_ref(), &[99, 33]);
        assert_eq!(retained.as_ref(), &[22, 33]);
        assert_eq!(source.as_ref(), &[11, 22, 33, 44]);
        let owned_ptr = shard.as_ptr();
        shard.as_mut()[1] = 88;
        assert_eq!(shard.as_ptr(), owned_ptr, "subsequent writes reuse the copy");
    }

    /// `take` hands out capacity, never length: the caller appends every byte it
    /// will read back. Reusing a slot must keep the allocation and must not memset
    /// it (rustfs/backlog#1159).
    #[test]
    fn shard_buffer_pool_reuses_the_allocation_and_never_zeroes_it() {
        let mut pool = ShardBufferPool::new(2);
        let mut buf = pool.take(1, 16);
        assert_eq!(buf.len(), 0, "take yields an empty buffer; the caller appends");
        assert!(buf.capacity() >= 16);
        buf.extend_from_slice(&[42u8; 16]);
        let capacity = buf.capacity();
        let ptr = buf.as_ptr();

        pool.put(1, buf);
        assert_eq!(pool.stored_capacity(1), Some(capacity));

        let reused = pool.take(1, 8);
        assert_eq!(reused.len(), 0);
        assert!(reused.capacity() >= capacity, "the allocation must be reused, not reallocated");
        assert_eq!(reused.as_ptr(), ptr, "same allocation");
    }

    #[test]
    fn shard_buffer_pool_grows_for_sparse_slot_indexes() {
        let mut pool = ShardBufferPool::new(0);
        let buf = pool.take(3, 4);

        assert_eq!(buf.len(), 0);
        assert!(buf.capacity() >= 4);
        assert_eq!(pool.buffers.len(), 4);
    }

    #[test]
    fn workspace_reports_shard_len_and_pool_reserves_when_reused_slot_is_too_small() {
        let workspace = RustfsCodecDecodeWorkspace::new(37);
        assert_eq!(workspace.shard_len(), 37);

        let mut pool = ShardBufferPool::new(1);
        pool.put(0, Vec::with_capacity(2));
        let grown = pool.take(0, 8);

        assert_eq!(grown.len(), 0);
        assert!(grown.capacity() >= 8, "a too-small reused slot must grow to the requested capacity");
    }
}
