use crate::graph::nodes::StringId;
use crate::graph::string_interner::StringInterner;
use rustc_hash::FxHashMap;

/// Trigram index for fast fuzzy symbol search.
///
/// Each 3-character substring of a symbol name maps to the set of StringIds
/// that contain it. Searching intersects trigram posting lists and ranks by
/// match quality.
pub struct TrigramIndex {
    /// Trigram → list of StringIds containing that trigram.
    postings: FxHashMap<[u8; 3], Vec<StringId>>,
    /// All indexed StringIds (for reference).
    all_ids: Vec<StringId>,
}

impl TrigramIndex {
    /// Build a trigram index over the given symbol names.
    pub fn build(ids: &[StringId], strings: &StringInterner) -> Self {
        let mut postings: FxHashMap<[u8; 3], Vec<StringId>> = FxHashMap::default();

        for &id in ids {
            let s = strings.get(id);
            let lower = s.to_ascii_lowercase();
            let bytes = lower.as_bytes();
            if bytes.len() < 3 {
                continue;
            }
            // Deduplicate trigrams per string
            let mut seen = rustc_hash::FxHashSet::default();
            for window in bytes.windows(3) {
                let tri: [u8; 3] = [window[0], window[1], window[2]];
                if seen.insert(tri) {
                    postings.entry(tri).or_default().push(id);
                }
            }
        }

        Self {
            postings,
            all_ids: ids.to_vec(),
        }
    }

    /// Search for symbols matching the query string.
    /// Returns StringIds ranked by match quality (best first).
    pub fn search(&self, query: &str, strings: &StringInterner) -> Vec<StringId> {
        let lower_query = query.to_ascii_lowercase();
        let query_bytes = lower_query.as_bytes();

        if query_bytes.len() < 3 {
            // For very short queries, fall back to substring match on all strings
            let mut results: Vec<StringId> = self
                .all_ids
                .iter()
                .filter(|&&id| {
                    strings
                        .get(id)
                        .to_ascii_lowercase()
                        .contains(&lower_query)
                })
                .copied()
                .collect();
            results.sort_unstable();
            results.dedup();
            return results;
        }

        // Extract query trigrams
        let mut query_trigrams = Vec::new();
        for window in query_bytes.windows(3) {
            let tri: [u8; 3] = [window[0], window[1], window[2]];
            query_trigrams.push(tri);
        }
        query_trigrams.sort_unstable();
        query_trigrams.dedup();

        if query_trigrams.is_empty() {
            return Vec::new();
        }

        // Count how many query trigrams each candidate matches
        let mut candidate_scores: FxHashMap<StringId, u32> = FxHashMap::default();
        for tri in &query_trigrams {
            if let Some(ids) = self.postings.get(tri) {
                for &id in ids {
                    *candidate_scores.entry(id).or_insert(0) += 1;
                }
            }
        }

        if candidate_scores.is_empty() {
            return Vec::new();
        }

        // Filter: candidates must match at least a quarter of query trigrams
        let min_trigrams = std::cmp::max(1, query_trigrams.len() as u32 / 4);
        let mut candidates: Vec<(StringId, u32)> = candidate_scores
            .into_iter()
            .filter(|&(_, count)| count >= min_trigrams)
            .collect();

        // Score each candidate: exact match > prefix > substring > trigram-only
        let score = |id: StringId, trigram_count: u32| -> (u32, u32, u32) {
            let s = strings.get(id).to_ascii_lowercase();
            let exact = if s == lower_query { 3 } else { 0 };
            let prefix = if exact == 0 && s.starts_with(&lower_query) {
                2
            } else {
                0
            };
            let substring = if exact == 0 && prefix == 0 && s.contains(&lower_query) {
                1
            } else {
                0
            };
            let quality = exact + prefix + substring;
            // Sort key: (quality desc, trigram_count desc, length asc for tiebreak)
            (quality, trigram_count, u32::MAX - s.len() as u32)
        };

        candidates.sort_unstable_by(|a, b| {
            let sa = score(a.0, a.1);
            let sb = score(b.0, b.1);
            sb.cmp(&sa)
        });

        candidates.into_iter().map(|(id, _)| id).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn build_test_index(names: &[&str]) -> (StringInterner, Vec<StringId>, TrigramIndex) {
        let mut strings = StringInterner::new();
        let ids: Vec<StringId> = names.iter().map(|s| strings.intern(s)).collect();
        let index = TrigramIndex::build(&ids, &strings);
        (strings, ids, index)
    }

    #[test]
    fn trigram_index_build() {
        // TEST trigram_index_build from ARCHITECTURE.md:
        // Build trigram index over 10K symbol names.
        // Every 3-character substring maps to the correct StringIds.
        let mut strings = StringInterner::new();
        let ids: Vec<StringId> = (0..10_000)
            .map(|i| strings.intern(&format!("symbol_{:05}", i)))
            .collect();

        let index = TrigramIndex::build(&ids, &strings);

        // Check that the trigram "sym" maps to all symbols
        let sym_tri: [u8; 3] = [b's', b'y', b'm'];
        let posting = index.postings.get(&sym_tri).unwrap();
        assert_eq!(posting.len(), 10_000);

        // Check a more specific trigram
        let tri_000: [u8; 3] = [b'0', b'0', b'0'];
        let posting = index.postings.get(&tri_000);
        assert!(posting.is_some());
        // "00000" through "00009" contain "000"
        assert!(posting.unwrap().len() >= 10);
    }

    #[test]
    fn trigram_search_exact() {
        // TEST trigram_search_exact from ARCHITECTURE.md
        let names = ["handleAudioStream", "handleVideoStream", "processAudio"];
        let (strings, _ids, index) = build_test_index(&names);

        let results = index.search("handleAudio", &strings);

        assert!(!results.is_empty());

        let result_names: Vec<&str> = results.iter().map(|&id| strings.get(id)).collect();

        // "handleAudioStream" ranked first (exact substring match)
        assert_eq!(result_names[0], "handleAudioStream");

        // "processAudio" should be in results (partial match on "Audio")
        assert!(result_names.contains(&"processAudio"));

        // "handleVideoStream" should be in results (matches "handle")
        assert!(result_names.contains(&"handleVideoStream"));

        // handleAudioStream should rank above processAudio (more trigram matches)
        let audio_stream_pos = result_names
            .iter()
            .position(|&n| n == "handleAudioStream")
            .unwrap();
        let process_audio_pos = result_names
            .iter()
            .position(|&n| n == "processAudio")
            .unwrap();
        assert!(audio_stream_pos < process_audio_pos);
    }

    #[test]
    fn trigram_search_case_insensitive() {
        // TEST trigram_search_case_insensitive from ARCHITECTURE.md
        let names = ["StreamingRecognize"];
        let (strings, _ids, index) = build_test_index(&names);

        let results = index.search("streamingrecognize", &strings);
        assert!(!results.is_empty());

        let result_names: Vec<&str> = results.iter().map(|&id| strings.get(id)).collect();
        assert!(result_names.contains(&"StreamingRecognize"));
    }

    #[test]
    fn trigram_search_no_results() {
        // TEST trigram_search_no_results from ARCHITECTURE.md
        let names = ["handleAudioStream", "processAudio"];
        let (strings, _ids, index) = build_test_index(&names);

        let results = index.search("xyzzyplugh", &strings);
        assert!(results.is_empty());
    }
}
