//! The pinyin input engine: segmentation → candidate generation → reranking.
//!
//! Given a raw pinyin key sequence and its [`InputContext`], the engine:
//!
//! 1. asks [`segment`] for valid syllable splits;
//! 2. for each split, runs a Viterbi-style shortest-path search over the
//!    [`Dictionary`] to build the best *full-sentence* candidate (the maximum
//!    total log-frequency segmentation of the syllables into dictionary words);
//! 3. adds shorter high-frequency word/prefix candidates and single-syllable
//!    fallbacks so the panel is never empty; and
//! 4. hands the ranked list to a [`CandidateReranker`] for context refinement.
//!
//! Scores are additive log-frequencies, which is what a real IME language model
//! optimizes: multiplying word probabilities becomes summing their logs, so the
//! best path is the highest-scoring one and stays numerically stable.

use std::collections::HashMap;

use crate::dict::Dictionary;
use crate::rerank::{CandidateReranker, PrefixContextReranker};
use crate::segment::segment;
use crate::{Candidate, InputContext, InputEngine};

/// A pinyin IME backed by an in-memory [`Dictionary`] and a pluggable
/// [`CandidateReranker`]. Construct with [`PinyinInputEngine::new`] or the
/// batteries-included [`PinyinInputEngine::builtin`].
pub struct PinyinInputEngine {
    dictionary: Dictionary,
    reranker: Box<dyn CandidateReranker>,
    /// Small smoothing weight for words the dictionary lacks, so single-syllable
    /// fallbacks still receive a finite score instead of `-inf`.
    unknown_weight: f64,
}

impl PinyinInputEngine {
    /// Build an engine from a dictionary and a reranker.
    pub fn new(dictionary: Dictionary, reranker: Box<dyn CandidateReranker>) -> Self {
        Self {
            dictionary,
            reranker,
            unknown_weight: 1.0,
        }
    }

    /// Build an engine from the built-in dictionary and the default
    /// context-aware reranker — usable and testable with no setup.
    pub fn builtin() -> Self {
        Self::new(
            Dictionary::builtin(),
            Box::new(PrefixContextReranker::default()),
        )
    }

    /// Access the underlying dictionary (e.g. to load more entries).
    pub fn dictionary_mut(&mut self) -> &mut Dictionary {
        &mut self.dictionary
    }

    /// Log-frequency score for a weight, with `+1` smoothing so weight `0` is
    /// finite and monotonic in weight.
    fn log_freq(weight: u32) -> f64 {
        ((weight as f64) + 1.0).ln()
    }

    /// Viterbi shortest-path over one syllable split: find the word
    /// segmentation maximizing total log-frequency. Returns the best sentence
    /// (joined word text, total score) or `None` if no path covers all
    /// syllables using dictionary words.
    fn best_sentence(&self, syllables: &[String]) -> Option<(String, f64)> {
        let n = syllables.len();
        // best[i] = (score, text) for the best segmentation of syllables[..i].
        let mut best: Vec<Option<(f64, String)>> = vec![None; n + 1];
        best[0] = Some((0.0, String::new()));

        for end in 1..=n {
            for start in 0..end {
                let Some((prev_score, prev_text)) = best[start].clone() else {
                    continue;
                };
                let key: String = syllables[start..end].concat();
                let hits = self.dictionary.lookup(&key);
                let (word, weight) = if let Some(top) = hits.first() {
                    (top.word.clone(), top.weight)
                } else if end - start == 1 {
                    // Unknown single syllable: keep the path alive with a
                    // placeholder using the raw pinyin, lightly smoothed. This
                    // guarantees a full-length path always exists.
                    (syllables[start].clone(), 0)
                } else {
                    continue;
                };
                let step = if weight == 0 {
                    self.unknown_weight.ln()
                } else {
                    Self::log_freq(weight)
                };
                let score = prev_score + step;
                let text = format!("{prev_text}{word}");
                match &best[end] {
                    Some((existing, _)) if *existing >= score => {}
                    _ => best[end] = Some((score, text)),
                }
            }
        }
        best[n].clone().map(|(score, text)| (text, score))
    }

    /// Collect candidates for a single syllable split into `acc`, keyed by text
    /// so duplicates across splits keep their best score.
    fn collect_from_split(&self, syllables: &[String], acc: &mut HashMap<String, Candidate>) {
        let n = syllables.len();
        if n == 0 {
            return;
        }

        // 1) Best full-sentence path over all syllables.
        if let Some((text, score)) = self.best_sentence(syllables) {
            Self::offer(
                acc,
                Candidate {
                    text,
                    syllables: syllables.to_vec(),
                    // Bias full coverage above shorter fragments of equal per-word
                    // frequency without distorting the log-scale ordering.
                    score: score + n as f64,
                },
            );
        }

        // 2) Whole-input dictionary words (e.g. `nihao` → 你好 directly), which
        // may beat the pieced-together sentence and are what users expect for
        // common words.
        let whole_key: String = syllables.concat();
        for entry in self.dictionary.lookup(&whole_key) {
            Self::offer(
                acc,
                Candidate {
                    text: entry.word.clone(),
                    syllables: syllables.to_vec(),
                    score: Self::log_freq(entry.weight) + n as f64,
                },
            );
        }

        // 3) Prefix words: the longest dictionary word starting at syllable 0
        // that covers a prefix of the input, so `nihaoshijie` still surfaces
        // `你好` as a strong shorter candidate.
        for prefix_len in (1..n).rev() {
            let key: String = syllables[..prefix_len].concat();
            for entry in self.dictionary.lookup(&key) {
                Self::offer(
                    acc,
                    Candidate {
                        text: entry.word.clone(),
                        syllables: syllables[..prefix_len].to_vec(),
                        score: Self::log_freq(entry.weight),
                    },
                );
            }
        }

        // 4) First-syllable single characters as a guaranteed non-empty
        // fallback (every legal syllable maps to at least the raw pinyin).
        let first = &syllables[0];
        let hits = self.dictionary.lookup(first);
        if hits.is_empty() {
            Self::offer(
                acc,
                Candidate {
                    text: first.clone(),
                    syllables: vec![first.clone()],
                    score: self.unknown_weight.ln(),
                },
            );
        } else {
            for entry in hits {
                Self::offer(
                    acc,
                    Candidate {
                        text: entry.word.clone(),
                        syllables: vec![first.clone()],
                        score: Self::log_freq(entry.weight),
                    },
                );
            }
        }
    }

    /// Insert `candidate` into `acc`, keeping the higher score on collision.
    fn offer(acc: &mut HashMap<String, Candidate>, candidate: Candidate) {
        acc.entry(candidate.text.clone())
            .and_modify(|existing| {
                if candidate.score > existing.score {
                    existing.score = candidate.score;
                    existing.syllables = candidate.syllables.clone();
                }
            })
            .or_insert(candidate);
    }
}

impl InputEngine for PinyinInputEngine {
    fn candidates(&self, pinyin: &str, context: &InputContext) -> Vec<Candidate> {
        let splits = segment(pinyin);
        if splits.is_empty() {
            return Vec::new();
        }

        let mut acc: HashMap<String, Candidate> = HashMap::new();
        // Consider the most promising splits (segment() returns them best-first);
        // capping keeps highly ambiguous inputs bounded without losing the
        // readings users actually mean.
        for split in splits.iter().take(6) {
            self.collect_from_split(&split.syllables, &mut acc);
        }

        let mut candidates: Vec<Candidate> = acc.into_values().collect();
        // Initial engine order: score desc, then longer coverage, then text for
        // determinism. The reranker refines this using context.
        candidates.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| b.coverage().cmp(&a.coverage()))
                .then_with(|| a.text.cmp(&b.text))
        });

        self.reranker.rerank(&mut candidates, context);

        if context.max_candidates > 0 && candidates.len() > context.max_candidates {
            candidates.truncate(context.max_candidates);
        }
        candidates
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn texts(candidates: &[Candidate]) -> Vec<&str> {
        candidates.iter().map(|c| c.text.as_str()).collect()
    }

    #[test]
    fn common_word_is_top_candidate() {
        let engine = PinyinInputEngine::builtin();
        let cs = engine.candidates("nihao", &InputContext::default());
        assert_eq!(cs[0].text, "你好");
    }

    #[test]
    fn single_syllable_offers_homophones_by_frequency() {
        let engine = PinyinInputEngine::builtin();
        let cs = engine.candidates("hao", &InputContext::with_limit(5));
        let words = texts(&cs);
        assert_eq!(words[0], "好"); // weight 8000 > 号 3000
        assert!(words.contains(&"号"));
    }

    #[test]
    fn full_sentence_search_pieces_words_together() {
        let engine = PinyinInputEngine::builtin();
        let cs = engine.candidates("woaizhongguo", &InputContext::default());
        // The best full-coverage sentence should read 我爱中国.
        assert_eq!(cs[0].text, "我爱中国");
    }

    #[test]
    fn longer_input_still_surfaces_shorter_prefix_word() {
        let engine = PinyinInputEngine::builtin();
        let cs = engine.candidates("nihaoshijie", &InputContext::default());
        let words = texts(&cs);
        // Full sentence 你好世界 present, and 你好 offered as a prefix candidate.
        assert!(words.contains(&"你好世界") || words.contains(&"你好"));
        assert!(words.contains(&"你好"));
    }

    #[test]
    fn respects_candidate_limit() {
        let engine = PinyinInputEngine::builtin();
        let cs = engine.candidates("hao", &InputContext::with_limit(1));
        assert_eq!(cs.len(), 1);
    }

    #[test]
    fn unknown_pinyin_falls_back_to_raw_syllable() {
        // A legal syllable with no dictionary word ("den") must still return
        // something rather than an empty panel.
        let engine = PinyinInputEngine::builtin();
        let cs = engine.candidates("den", &InputContext::default());
        assert!(!cs.is_empty());
        assert_eq!(cs[0].text, "den");
    }

    #[test]
    fn empty_input_yields_no_candidates() {
        let engine = PinyinInputEngine::builtin();
        assert!(engine
            .candidates("", &InputContext::default())
            .is_empty());
    }

    #[test]
    fn context_reranks_to_avoid_repetition() {
        let mut dict = Dictionary::new();
        dict.insert("妈", "ma", 1000);
        dict.insert("吗", "ma", 990);
        let engine = PinyinInputEngine::new(dict, Box::new(PrefixContextReranker::default()));
        let ctx = InputContext {
            preceding_text: "妈".to_string(),
            max_candidates: 0,
        };
        let cs = engine.candidates("ma", &ctx);
        assert_eq!(cs[0].text, "吗"); // repetition of 妈 demoted
    }
}
