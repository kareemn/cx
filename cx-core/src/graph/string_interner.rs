use super::nodes::StringId;
use rustc_hash::FxHashMap;

/// PERFORMANCE CRITICAL: String interning eliminates all string allocations
/// during graph traversal and all string comparisons during queries.
///
/// Packed format: [len:u32][bytes...][len:u32][bytes...]...
/// Each StringId is the byte offset of the length prefix in `data`.
#[derive(Debug)]
pub struct StringInterner {
    /// Packed string data.
    data: Vec<u8>,
    /// Hash of string content → StringId (byte offset). For dedup during intern.
    lookup: FxHashMap<u64, StringId>,
}

impl StringInterner {
    /// Create a new empty interner.
    pub fn new() -> Self {
        Self {
            data: Vec::new(),
            lookup: FxHashMap::default(),
        }
    }

    /// Create an interner with pre-allocated capacity for the data buffer.
    pub fn with_capacity(bytes: usize) -> Self {
        Self {
            data: Vec::with_capacity(bytes),
            lookup: FxHashMap::default(),
        }
    }

    /// Intern a string — returns existing ID if already present.
    pub fn intern(&mut self, s: &str) -> StringId {
        let hash = Self::hash_str(s);
        if let Some(&id) = self.lookup.get(&hash) {
            // Verify it's actually the same string (hash collision check)
            if self.get(id) == s {
                return id;
            }
            // Hash collision — fall through to insert with a different slot.
            // For simplicity, use linear probing on the hash.
            let mut probe = hash.wrapping_add(1);
            loop {
                if let Some(&existing_id) = self.lookup.get(&probe) {
                    if self.get(existing_id) == s {
                        return existing_id;
                    }
                    probe = probe.wrapping_add(1);
                } else {
                    return self.insert_at_hash(probe, s);
                }
            }
        }
        self.insert_at_hash(hash, s)
    }

    fn insert_at_hash(&mut self, hash: u64, s: &str) -> StringId {
        let id = self.data.len() as StringId;
        let len = s.len() as u32;
        self.data.extend_from_slice(&len.to_le_bytes());
        self.data.extend_from_slice(s.as_bytes());
        self.lookup.insert(hash, id);
        id
    }

    /// Get string content by ID — the ID is the byte offset of the length prefix.
    pub fn get(&self, id: StringId) -> &str {
        let offset = id as usize;
        let len_bytes: [u8; 4] = self.data[offset..offset + 4]
            .try_into()
            .expect("invalid StringId");
        let len = u32::from_le_bytes(len_bytes) as usize;
        let start = offset + 4;
        std::str::from_utf8(&self.data[start..start + len]).expect("invalid UTF-8 in string table")
    }

    /// Return the number of interned strings.
    pub fn len(&self) -> usize {
        self.lookup.len()
    }

    /// Return whether the interner is empty.
    pub fn is_empty(&self) -> bool {
        self.lookup.is_empty()
    }

    /// Return the raw packed data bytes (for serialization).
    pub fn data(&self) -> &[u8] {
        &self.data
    }

    /// Reconstruct lookup table from packed data (for deserialization / mmap load).
    pub fn from_data(data: Vec<u8>) -> Self {
        let mut lookup = FxHashMap::default();
        let mut offset = 0usize;
        while offset + 4 <= data.len() {
            let len_bytes: [u8; 4] = data[offset..offset + 4]
                .try_into()
                .expect("truncated string table");
            let len = u32::from_le_bytes(len_bytes) as usize;
            let start = offset + 4;
            if start + len > data.len() {
                break;
            }
            let s = std::str::from_utf8(&data[start..start + len])
                .expect("invalid UTF-8 in string table");
            let hash = Self::hash_str(s);
            // Handle hash collisions on rebuild
            let mut probe = hash;
            while lookup.contains_key(&probe) {
                probe = probe.wrapping_add(1);
            }
            lookup.insert(probe, offset as StringId);
            offset = start + len;
        }
        Self { data, lookup }
    }

    fn hash_str(s: &str) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut hasher = rustc_hash::FxHasher::default();
        s.hash(&mut hasher);
        hasher.finish()
    }
}

impl Default for StringInterner {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn string_interner_dedup() {
        // TEST string_interner_dedup from ARCHITECTURE.md:
        // Intern "foo" twice. Get same StringId both times.
        let mut interner = StringInterner::new();
        let id1 = interner.intern("foo");
        let id2 = interner.intern("foo");
        assert_eq!(id1, id2);

        // Intern 100K unique strings. Verify all retrievable by ID.
        let mut ids = Vec::with_capacity(100_000);
        for i in 0..100_000u32 {
            let s = format!("string_{}", i);
            ids.push((interner.intern(&s), s));
        }

        // No duplicate IDs, all strings round-trip correctly
        for (id, expected) in &ids {
            assert_eq!(interner.get(*id), expected.as_str());
        }

        // Verify dedup: interning again returns the same IDs
        for (id, s) in &ids {
            assert_eq!(interner.intern(s), *id);
        }
    }

    #[test]
    fn string_interner_roundtrip() {
        // TEST string_interner_roundtrip from ARCHITECTURE.md:
        // Intern 100K strings. Serialize to packed format. Reload.
        // All 100K strings retrievable by original ID.
        let mut interner = StringInterner::new();
        let mut entries = Vec::with_capacity(100_000);

        for i in 0..100_000u32 {
            let s = format!("sym_{}", i);
            let id = interner.intern(&s);
            entries.push((id, s));
        }

        // Serialize: just the raw data
        let data = interner.data().to_vec();

        // Reload from packed data
        let reloaded = StringInterner::from_data(data);

        // All 100K strings retrievable by original ID
        for (id, expected) in &entries {
            assert_eq!(reloaded.get(*id), expected.as_str());
        }
    }

    #[test]
    fn empty_string() {
        let mut interner = StringInterner::new();
        let id = interner.intern("");
        assert_eq!(interner.get(id), "");
    }

    #[test]
    fn unicode_strings() {
        let mut interner = StringInterner::new();
        let id = interner.intern("héllo wörld 🦀");
        assert_eq!(interner.get(id), "héllo wörld 🦀");
    }

    #[test]
    fn different_strings_get_different_ids() {
        let mut interner = StringInterner::new();
        let id1 = interner.intern("alpha");
        let id2 = interner.intern("beta");
        assert_ne!(id1, id2);
        assert_eq!(interner.get(id1), "alpha");
        assert_eq!(interner.get(id2), "beta");
    }

    #[test]
    fn len_tracking() {
        let mut interner = StringInterner::new();
        assert!(interner.is_empty());
        assert_eq!(interner.len(), 0);

        interner.intern("a");
        assert_eq!(interner.len(), 1);

        interner.intern("a"); // dedup
        assert_eq!(interner.len(), 1);

        interner.intern("b");
        assert_eq!(interner.len(), 2);
    }
}
