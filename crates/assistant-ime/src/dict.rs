//! Dictionary: the word ↔ pinyin ↔ frequency data the engine searches.
//!
//! This is the pure-Rust stand-in for what rime-ice provides to librime: a set
//! of entries, each mapping a Chinese word to its space-free pinyin key and a
//! frequency weight. The engine looks entries up by their exact pinyin key
//! (e.g. `nihao` → 你好); the segmenter has already decided the syllable
//! boundaries, so the dictionary only needs exact key matching plus fuzzy
//! normalization.
//!
//! For real use, [`Dictionary::load_text`] ingests a rime-ice-style table
//! (`word<TAB>pinyin<TAB>weight`, `#` comments allowed). A compact built-in
//! dictionary ([`Dictionary::builtin`]) keeps the engine usable and fully
//! unit-testable offline.

use std::collections::HashMap;

/// One dictionary record: a word, its toneless pinyin key with syllables joined
/// (no spaces), and a frequency weight. Higher weight ranks earlier.
#[derive(Debug, Clone, PartialEq)]
pub struct DictionaryEntry {
    pub word: String,
    /// Pinyin syllables joined without separators, e.g. `nihao`. This is the
    /// key the engine looks up.
    pub pinyin: String,
    pub weight: u32,
}

/// A frequency-weighted pinyin dictionary with exact-key and fuzzy lookup.
#[derive(Debug, Clone, Default)]
pub struct Dictionary {
    /// Exact toneless pinyin key → entries sharing that key (homophones).
    by_pinyin: HashMap<String, Vec<DictionaryEntry>>,
    /// Whether common fuzzy-pinyin equivalences are applied on lookup.
    fuzzy: bool,
}

impl Dictionary {
    /// An empty dictionary. Fuzzy matching is on by default because Chinese
    /// users routinely rely on `zh/z`, `in/ing` tolerance.
    pub fn new() -> Self {
        Self {
            by_pinyin: HashMap::new(),
            fuzzy: true,
        }
    }

    /// Enable or disable fuzzy-pinyin equivalences (`zh↔z`, `ch↔c`, `sh↔s`,
    /// `in↔ing`, `en↔eng`, `l↔n`, `v↔u`). Returns `self` for chaining.
    pub fn with_fuzzy(mut self, fuzzy: bool) -> Self {
        self.fuzzy = fuzzy;
        self
    }

    /// Insert one entry, keeping entries under the same key sorted by
    /// descending weight so the most frequent homophone is first.
    pub fn insert(&mut self, word: impl Into<String>, pinyin: impl Into<String>, weight: u32) {
        let entry = DictionaryEntry {
            word: word.into(),
            pinyin: normalize_key(&pinyin.into()),
            weight,
        };
        let bucket = self.by_pinyin.entry(entry.pinyin.clone()).or_default();
        bucket.push(entry);
        bucket.sort_by(|a, b| b.weight.cmp(&a.weight).then_with(|| a.word.cmp(&b.word)));
    }

    /// Number of distinct pinyin keys held.
    pub fn key_count(&self) -> usize {
        self.by_pinyin.len()
    }

    /// Whether the dictionary holds no entries.
    pub fn is_empty(&self) -> bool {
        self.by_pinyin.is_empty()
    }

    /// Exact lookup of all words whose pinyin key equals `key` (already
    /// syllable-joined), best-weighted first. With fuzzy on, keys that differ
    /// only by a tolerated equivalence also match.
    pub fn lookup(&self, key: &str) -> Vec<&DictionaryEntry> {
        let key = normalize_key(key);
        let mut out: Vec<&DictionaryEntry> = Vec::new();
        if let Some(bucket) = self.by_pinyin.get(&key) {
            out.extend(bucket.iter());
        }
        if self.fuzzy {
            let canon = fuzzy_canon(&key);
            for (candidate, bucket) in &self.by_pinyin {
                if candidate == &key {
                    continue;
                }
                if fuzzy_canon(candidate) == canon {
                    out.extend(bucket.iter());
                }
            }
        }
        out.sort_by(|a, b| b.weight.cmp(&a.weight).then_with(|| a.word.cmp(&b.word)));
        out.dedup_by(|a, b| a.word == b.word && a.pinyin == b.pinyin);
        out
    }

    /// Load entries from a rime-ice-style text table. Each non-empty,
    /// non-comment line is `word<TAB or spaces>pinyin[<TAB or spaces>weight]`.
    /// A missing weight defaults to `1`; malformed lines are skipped. Returns
    /// the number of entries added.
    pub fn load_text(&mut self, text: &str) -> usize {
        let mut added = 0;
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let mut cols = line.split_whitespace();
            let (Some(word), Some(pinyin)) = (cols.next(), cols.next()) else {
                continue;
            };
            let weight = cols.next().and_then(|w| w.parse::<u32>().ok()).unwrap_or(1);
            self.insert(word, pinyin, weight);
            added += 1;
        }
        added
    }

    /// A compact, hand-tuned built-in dictionary. It is deliberately small but
    /// covers the words used across the crate's tests and demos, so the engine
    /// produces meaningful multi-word candidates offline. Weights are relative.
    pub fn builtin() -> Self {
        let mut dict = Dictionary::new();
        // (word, pinyin, weight) — weights loosely reflect real frequency so
        // ranking behaves plausibly in tests.
        const ENTRIES: &[(&str, &str, u32)] = &[
            ("你", "ni", 9000),
            ("你好", "nihao", 5000),
            ("好", "hao", 8000),
            ("号", "hao", 3000),
            ("你们", "nimen", 2600),
            ("我", "wo", 9500),
            ("我们", "women", 4200),
            ("爱", "ai", 3000),
            ("中", "zhong", 4000),
            ("中国", "zhongguo", 5200),
            ("国", "guo", 3500),
            ("世界", "shijie", 3800),
            ("世", "shi", 1500),
            ("界", "jie", 1200),
            ("先", "xian", 2600),
            ("西安", "xian", 1800),
            ("现在", "xianzai", 2400),
            ("输入", "shuru", 2200),
            ("输入法", "shurufa", 2600),
            ("方法", "fangfa", 2000),
            ("法", "fa", 1400),
            ("是", "shi", 9800),
            ("的", "de", 12000),
            ("测试", "ceshi", 2100),
            ("智能", "zhineng", 2300),
            ("助手", "zhushou", 2000),
            ("拼音", "pinyin", 2500),
            ("候选", "houxuan", 1600),
            ("词", "ci", 1700),
            ("今天", "jintian", 3100),
            ("天气", "tianqi", 2400),
            ("很", "hen", 5200),
            ("不错", "bucuo", 2200),
            ("谢谢", "xiexie", 3000),
        ];
        for &(word, pinyin, weight) in ENTRIES {
            dict.insert(word, pinyin, weight);
        }
        dict
    }
}

/// Normalize a pinyin key for storage/lookup: lowercased, apostrophes and
/// whitespace removed (keys are syllable-joined), `ü`→`v`.
fn normalize_key(key: &str) -> String {
    key.chars()
        .filter(|c| !c.is_whitespace() && *c != '\'')
        .map(|c| match c {
            'ü' => 'v',
            other => other.to_ascii_lowercase(),
        })
        .collect()
}

/// Collapse a key to a fuzzy canonical form so tolerated variants compare
/// equal. Order matters: multi-char rules first. This is intentionally simple
/// and total (no panics), matching how mainstream IMEs offer opt-in fuzzy sets.
fn fuzzy_canon(key: &str) -> String {
    let mut s = key.to_string();
    // Retroflex → dental initials.
    s = s.replace("zh", "z").replace("ch", "c").replace("sh", "s");
    // Nasal finals.
    s = s.replace("ing", "in").replace("eng", "en").replace("ang", "an");
    // l/n initial confusion and ü spelling.
    s = s.replace('l', "n").replace('v', "u");
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn insert_and_exact_lookup_orders_by_weight() {
        let mut dict = Dictionary::new().with_fuzzy(false);
        dict.insert("号", "hao", 3000);
        dict.insert("好", "hao", 8000);
        let hits = dict.lookup("hao");
        assert_eq!(hits[0].word, "好"); // higher weight first
        assert_eq!(hits[1].word, "号");
    }

    #[test]
    fn homophones_share_a_key() {
        let dict = Dictionary::builtin();
        let hits = dict.lookup("xian");
        let words: Vec<&str> = hits.iter().map(|e| e.word.as_str()).collect();
        assert!(words.contains(&"先"));
        assert!(words.contains(&"西安"));
    }

    #[test]
    fn fuzzy_matches_retroflex_and_nasal() {
        let mut dict = Dictionary::new(); // fuzzy on
        dict.insert("中国", "zhongguo", 100);
        // Typed without the `h` and with the wrong nasal: still found via fuzzy.
        assert!(!dict.lookup("zongguo").is_empty());
    }

    #[test]
    fn fuzzy_off_is_strict() {
        let mut dict = Dictionary::new().with_fuzzy(false);
        dict.insert("中国", "zhongguo", 100);
        assert!(dict.lookup("zongguo").is_empty());
        assert!(!dict.lookup("zhongguo").is_empty());
    }

    #[test]
    fn load_text_parses_table_and_skips_junk() {
        let mut dict = Dictionary::new();
        let added = dict.load_text(
            "# comment\n你好\tnihao\t5000\n好 hao 8000\n\nmalformed_no_pinyin\n世界 shijie",
        );
        assert_eq!(added, 3); // 你好, 好, 世界 (last defaults weight 1)
        assert_eq!(dict.lookup("nihao")[0].word, "你好");
        assert_eq!(dict.lookup("shijie")[0].weight, 1);
    }

    #[test]
    fn normalize_key_strips_separators_and_lowercases() {
        assert_eq!(normalize_key("Ni'Hao"), "nihao");
        assert_eq!(normalize_key("l ü"), "lv");
    }

    #[test]
    fn builtin_is_non_empty() {
        assert!(!Dictionary::builtin().is_empty());
        assert!(Dictionary::builtin().key_count() > 20);
    }
}
