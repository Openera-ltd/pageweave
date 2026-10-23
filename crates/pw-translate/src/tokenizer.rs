//! SentencePiece Unigram tokenization for Marian (OPUS-MT) models, read from the model's
//! `tokenizer.json`: NFKC normalization, whitespace split, "▁" word prefix, best-scoring
//! segmentation (Viterbi) over the vocabulary; unknown characters become `<unk>`.

use pw_core::{ErrorCategory, PwError, PwResult};
use std::collections::HashMap;
use unicode_normalization::UnicodeNormalization;

pub const WORD: char = '\u{2581}';

pub struct Tokenizer {
    ids: HashMap<String, (u32, f64)>,
    pieces: Vec<String>,
    unk: u32,
    unk_score: f64,
    max_len: usize,
    /// Ids that are never written out when decoding (end of sentence, padding, unknown).
    special: Vec<u32>,
}

fn bad(what: &str) -> PwError {
    PwError::new(
        ErrorCategory::CorruptResource,
        "translate.tokenizer",
        format!("The translation vocabulary is damaged ({what})."),
    )
}

impl Tokenizer {
    pub fn from_json(text: &str) -> PwResult<Tokenizer> {
        let v: serde_json::Value = serde_json::from_str(text).map_err(|_| bad("not JSON"))?;
        let model = &v["model"];
        if model["type"] != "Unigram" {
            return Err(bad("not a Unigram model"));
        }
        let vocab = model["vocab"].as_array().ok_or_else(|| bad("no vocabulary"))?;
        let unk = model["unk_id"].as_u64().ok_or_else(|| bad("no unknown id"))? as u32;
        let mut ids = HashMap::with_capacity(vocab.len());
        let mut pieces = Vec::with_capacity(vocab.len());
        let mut min_score = 0f64;
        let mut max_len = 1;
        for (i, e) in vocab.iter().enumerate() {
            let piece = e[0].as_str().ok_or_else(|| bad("piece"))?.to_string();
            let score = e[1].as_f64().ok_or_else(|| bad("score"))?;
            min_score = min_score.min(score);
            max_len = max_len.max(piece.chars().count());
            ids.entry(piece.clone()).or_insert((i as u32, score));
            pieces.push(piece);
        }
        let mut special: Vec<u32> = v["added_tokens"]
            .as_array()
            .map(|a| {
                a.iter().filter(|t| t["special"] == true).filter_map(|t| t["id"].as_u64()).map(|x| x as u32).collect()
            })
            .unwrap_or_default();
        special.push(unk);
        Ok(Tokenizer { ids, pieces, unk, unk_score: min_score - 10.0, max_len, special })
    }

    pub fn vocab_size(&self) -> usize {
        self.pieces.len()
    }

    pub fn id(&self, piece: &str) -> Option<u32> {
        self.ids.get(piece).map(|x| x.0)
    }

    /// Token ids of `text` (without the end-of-sentence id).
    pub fn encode(&self, text: &str) -> Vec<u32> {
        let norm: String = text.nfkc().collect();
        let mut out = Vec::new();
        for word in norm.split_whitespace() {
            let chars: Vec<char> = std::iter::once(WORD).chain(word.chars()).collect();
            let n = chars.len();
            // best[i]: best score of chars[..i] and where its last piece starts.
            let mut best: Vec<(f64, usize, u32)> = vec![(f64::NEG_INFINITY, 0, 0); n + 1];
            best[0].0 = 0.0;
            for i in 0..n {
                if best[i].0 == f64::NEG_INFINITY {
                    continue;
                }
                let mut piece = String::new();
                for j in i..n.min(i + self.max_len) {
                    piece.push(chars[j]);
                    if let Some(&(id, score)) = self.ids.get(&piece) {
                        let s = best[i].0 + score;
                        if s > best[j + 1].0 {
                            best[j + 1] = (s, i, id);
                        }
                    }
                }
                // A character no piece covers.
                let s = best[i].0 + self.unk_score;
                if s > best[i + 1].0 {
                    best[i + 1] = (s, i, self.unk);
                }
            }
            let mut ids = Vec::new();
            let mut i = n;
            while i > 0 {
                let (_, from, id) = best[i];
                // Consecutive unknowns are one <unk>, as SentencePiece does.
                if !(id == self.unk && ids.last() == Some(&self.unk)) {
                    ids.push(id);
                }
                i = from;
            }
            ids.reverse();
            out.extend(ids);
        }
        out
    }

    /// Text of token ids (special ids dropped, "▁" back to spaces).
    pub fn decode(&self, ids: &[u32]) -> String {
        let mut s = String::new();
        for &id in ids {
            if self.special.contains(&id) {
                continue;
            }
            if let Some(p) = self.pieces.get(id as usize) {
                s.push_str(p);
            }
        }
        s.replace(WORD, " ").trim().to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tok() -> Tokenizer {
        let j = serde_json::json!({
            "added_tokens": [{"id": 0, "content": "</s>", "special": true}, {"id": 1, "content": "<unk>", "special": true}, {"id": 9, "content": "<pad>", "special": true}],
            "model": {"type": "Unigram", "unk_id": 1, "vocab": [
                ["</s>", 0.0], ["<unk>", 0.0], ["▁the", -1.0], ["▁", -3.0], ["t", -4.0], ["he", -3.5], ["▁cat", -2.0], ["▁ca", -2.5], ["s", -2.0], ["<pad>", 0.0], ["x", -6.0]
            ]}
        });
        Tokenizer::from_json(&j.to_string()).expect("tok")
    }

    #[test]
    fn best_segmentation_unknowns_and_round_trip() {
        let t = tok();
        assert_eq!(t.encode("the cats"), vec![2, 6, 8]);
        // "é" is unknown: one <unk>; full-width letters are normalized (NFKC) first.
        assert_eq!(t.encode("the éé"), vec![2, 3, 1]);
        assert_eq!(t.encode("ｔhe"), vec![2]);
        assert_eq!(t.decode(&[2, 6, 8, 0, 9]), "the cats");
        assert_eq!(t.decode(&[1, 2]), "the");
        assert!(t.encode("   ").is_empty());
        assert!(Tokenizer::from_json("{}").is_err());
    }
}
