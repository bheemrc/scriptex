/// Lightweight English hyphenation using rule-based patterns.
/// Finds positions where a word can be broken with a hyphen.

/// Find the best hyphenation point that fits within `max_prefix_chars` characters.
/// `word` should be an ASCII word (bytes), may include trailing punctuation.
/// Returns the byte position to break at, or None if no valid break exists.
/// The caller emits word[..pos] + "-" on the current line and word[pos..] on the next.
#[inline]
pub fn best_break(word: &[u8], max_prefix_chars: usize) -> Option<usize> {
    // Strip trailing punctuation for analysis
    let mut wlen = word.len();
    while wlen > 0 && !word[wlen - 1].is_ascii_alphanumeric() {
        wlen -= 1;
    }

    if max_prefix_chars < 3 || wlen < 7 {
        return None;
    }

    let mut w = [0u8; 32];
    let n = wlen.min(32);
    for i in 0..n {
        w[i] = word[i].to_ascii_lowercase();
    }

    // Valid break range: at least 2 chars before and 3 chars after (in the alphabetic part)
    let min_pos = 2usize;
    let max_pos = n.saturating_sub(3);
    if min_pos > max_pos {
        return None;
    }

    // Score each position: higher = better break point
    let mut scores = [0i8; 32];

    // Rule 1: Break after common prefixes (highest priority)
    prefix_score(&w[..n], &mut scores, b"inter", 5);
    prefix_score(&w[..n], &mut scores, b"counter", 5);
    prefix_score(&w[..n], &mut scores, b"super", 4);
    prefix_score(&w[..n], &mut scores, b"under", 4);
    prefix_score(&w[..n], &mut scores, b"trans", 4);
    prefix_score(&w[..n], &mut scores, b"multi", 4);
    prefix_score(&w[..n], &mut scores, b"micro", 4);
    prefix_score(&w[..n], &mut scores, b"hyper", 4);
    prefix_score(&w[..n], &mut scores, b"ultra", 4);
    prefix_score(&w[..n], &mut scores, b"over", 4);
    prefix_score(&w[..n], &mut scores, b"auto", 4);
    prefix_score(&w[..n], &mut scores, b"anti", 4);
    prefix_score(&w[..n], &mut scores, b"dis", 4);
    prefix_score(&w[..n], &mut scores, b"pre", 4);
    prefix_score(&w[..n], &mut scores, b"un", 4);
    prefix_score(&w[..n], &mut scores, b"semi", 3);
    prefix_score(&w[..n], &mut scores, b"non", 3);
    prefix_score(&w[..n], &mut scores, b"sub", 3);
    prefix_score(&w[..n], &mut scores, b"mis", 3);
    prefix_score(&w[..n], &mut scores, b"post", 3);
    prefix_score(&w[..n], &mut scores, b"re", 3);
    prefix_score(&w[..n], &mut scores, b"de", 3);
    prefix_score(&w[..n], &mut scores, b"out", 3);
    prefix_score(&w[..n], &mut scores, b"con", 3);
    prefix_score(&w[..n], &mut scores, b"com", 3);
    prefix_score(&w[..n], &mut scores, b"pro", 3);
    prefix_score(&w[..n], &mut scores, b"para", 3);
    prefix_score(&w[..n], &mut scores, b"extra", 4);
    prefix_score(&w[..n], &mut scores, b"meta", 3);
    prefix_score(&w[..n], &mut scores, b"infra", 3);
    prefix_score(&w[..n], &mut scores, b"intra", 4);
    prefix_score(&w[..n], &mut scores, b"macro", 4);
    prefix_score(&w[..n], &mut scores, b"mono", 3);

    // Rule 2: Break before common suffixes
    suffix_score(&w[..n], &mut scores, b"tion", 5);
    suffix_score(&w[..n], &mut scores, b"sion", 5);
    suffix_score(&w[..n], &mut scores, b"ment", 4);
    suffix_score(&w[..n], &mut scores, b"ness", 4);
    suffix_score(&w[..n], &mut scores, b"able", 4);
    suffix_score(&w[..n], &mut scores, b"ible", 4);
    suffix_score(&w[..n], &mut scores, b"ture", 4);
    suffix_score(&w[..n], &mut scores, b"ence", 3);
    suffix_score(&w[..n], &mut scores, b"ance", 3);
    suffix_score(&w[..n], &mut scores, b"ical", 3);
    suffix_score(&w[..n], &mut scores, b"ally", 3);
    suffix_score(&w[..n], &mut scores, b"ling", 3);
    suffix_score(&w[..n], &mut scores, b"ious", 3);
    suffix_score(&w[..n], &mut scores, b"eous", 3);
    suffix_score(&w[..n], &mut scores, b"less", 3);
    suffix_score(&w[..n], &mut scores, b"ful", 3);
    suffix_score(&w[..n], &mut scores, b"ity", 3);
    suffix_score(&w[..n], &mut scores, b"ize", 3);
    suffix_score(&w[..n], &mut scores, b"ise", 3);
    suffix_score(&w[..n], &mut scores, b"ing", 2);
    suffix_score(&w[..n], &mut scores, b"ous", 2);
    suffix_score(&w[..n], &mut scores, b"ive", 2);
    suffix_score(&w[..n], &mut scores, b"lines", 4);
    suffix_score(&w[..n], &mut scores, b"ment", 3);
    suffix_score(&w[..n], &mut scores, b"ments", 4);
    suffix_score(&w[..n], &mut scores, b"tions", 5);
    suffix_score(&w[..n], &mut scores, b"sions", 5);
    suffix_score(&w[..n], &mut scores, b"ation", 5);
    suffix_score(&w[..n], &mut scores, b"atory", 4);
    suffix_score(&w[..n], &mut scores, b"ment", 4);
    suffix_score(&w[..n], &mut scores, b"ular", 3);
    suffix_score(&w[..n], &mut scores, b"ably", 3);
    suffix_score(&w[..n], &mut scores, b"ibly", 3);
    suffix_score(&w[..n], &mut scores, b"ery", 2);
    suffix_score(&w[..n], &mut scores, b"ory", 2);
    suffix_score(&w[..n], &mut scores, b"ary", 2);
    suffix_score(&w[..n], &mut scores, b"ical", 3);
    suffix_score(&w[..n], &mut scores, b"ally", 3);

    // Rule 3: Break between double consonants (run-ning, slip-per)
    for i in 1..n.saturating_sub(2) {
        if w[i] == w[i + 1] && is_consonant(w[i]) {
            let pos = i + 1;
            if pos >= min_pos && pos <= max_pos {
                scores[pos] = scores[pos].max(2);
            }
        }
    }

    // Rule 4: Syllable boundary patterns (lowest priority)
    for i in 1..n.saturating_sub(2) {
        // VC|CV — break between two consonants surrounded by vowels
        if i + 3 < n && is_vowel(w[i]) && is_consonant(w[i + 1]) && is_consonant(w[i + 2]) && is_vowel(w[i + 3]) {
            let pos = i + 2;
            if pos >= min_pos && pos <= max_pos {
                scores[pos] = scores[pos].max(1);
            }
        }
        // V|CV — break before single consonant between vowels (weakest rule)
        if i >= 2 && i + 1 < n && is_vowel(w[i - 1]) && is_consonant(w[i]) && is_vowel(w[i + 1]) {
            if i >= min_pos && i <= max_pos {
                // Only use this if no better break exists nearby
                if scores[i] == 0 {
                    scores[i] = 1;
                }
            }
        }
    }

    // Negative: don't break within consonant digraphs
    for i in 0..n.saturating_sub(1) {
        let a = w[i];
        let b = w[i + 1];
        if matches!((a, b), (b'c', b'k') | (b'g', b'h') | (b'p', b'h') | (b's', b'h') |
                    (b't', b'h') | (b'w', b'h') | (b'c', b'h') | (b'q', b'u') |
                    (b'g', b'n') | (b'k', b'n')) {
            if i + 1 < 32 { scores[i + 1] = -5; }
        }
    }

    // Morpheme-aware rules for common words with tricky breaks
    // Break "graph" words correctly: -graph- not -gra|ph-
    for i in 0..n.saturating_sub(4) {
        if w[i..].starts_with(b"graph") {
            // Prefer break before "graph", block break within it
            if i >= 2 && i < 32 { scores[i] = scores[i].max(5); }
            if i + 2 < 32 { scores[i + 2] = -5; } // don't break gra|ph
        }
    }

    // Find the best break point: among valid positions within max_prefix_chars,
    // prefer highest score, then prefer the one closest to ~55% of word length
    let limit = max_prefix_chars.min(max_pos);
    let ideal = (n as f32 * 0.55) as usize;
    let mut best: Option<usize> = None;
    let mut best_score = 0i8;
    let mut best_dist = usize::MAX;

    for i in min_pos..=limit {
        if scores[i] > 0 {
            let dist = if i > ideal { i - ideal } else { ideal - i };
            if scores[i] > best_score || (scores[i] == best_score && dist < best_dist) {
                best = Some(i);
                best_score = scores[i];
                best_dist = dist;
            }
        }
    }
    best
}

#[inline(always)]
fn is_vowel(b: u8) -> bool {
    matches!(b, b'a' | b'e' | b'i' | b'o' | b'u' | b'y')
}

#[inline(always)]
fn is_consonant(b: u8) -> bool {
    b >= b'a' && b <= b'z' && !matches!(b, b'a' | b'e' | b'i' | b'o' | b'u')
}

#[inline]
fn prefix_score(word: &[u8], scores: &mut [i8; 32], prefix: &[u8], score: i8) {
    let plen = prefix.len();
    if word.len() > plen + 2 && word[..plen] == *prefix {
        scores[plen] = scores[plen].max(score);
    }
}

#[inline]
fn suffix_score(word: &[u8], scores: &mut [i8; 32], suffix: &[u8], score: i8) {
    let slen = suffix.len();
    let wlen = word.len();
    if wlen > slen + 2 && word[wlen - slen..] == *suffix {
        let pos = wlen - slen;
        if pos < 32 && pos >= 2 {
            scores[pos] = scores[pos].max(score);
        }
    }
}
