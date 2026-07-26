/// Bits per machine word; the single source of truth for the whole crate.
pub(crate) const WORD_BITS: usize = usize::BITS as usize;

pub(crate) struct Bitmap<'a> {
    words: &'a mut [usize],
    bit_len: usize,
}

impl<'a> Bitmap<'a> {
    pub(crate) fn new(words: &'a mut [usize], bit_len: usize) -> Self {
        words.fill(0);
        Self { words, bit_len }
    }

    #[inline]
    pub(crate) fn get(&self, bit: usize) -> bool {
        debug_assert!(bit < self.bit_len);
        self.words[bit / WORD_BITS] & (1usize << (bit % WORD_BITS)) != 0
    }

    #[inline]
    pub(crate) fn set(&mut self, bit: usize) {
        debug_assert!(bit < self.bit_len);
        self.words[bit / WORD_BITS] |= 1usize << (bit % WORD_BITS);
    }

    #[inline]
    pub(crate) fn clear(&mut self, bit: usize) {
        debug_assert!(bit < self.bit_len);
        self.words[bit / WORD_BITS] &= !(1usize << (bit % WORD_BITS));
    }

    pub(crate) fn find_first_set(&self, start: usize, end: usize) -> Option<usize> {
        debug_assert!(start <= end);
        debug_assert!(end <= self.bit_len);
        if start == end {
            return None;
        }

        let first_word = start / WORD_BITS;
        let last_word = (end - 1) / WORD_BITS;

        for word_index in first_word..=last_word {
            let word_start = word_index * WORD_BITS;
            let lower_bit = start.saturating_sub(word_start).min(WORD_BITS);
            let upper_bit = end.saturating_sub(word_start).min(WORD_BITS);

            let lower_mask = usize::MAX << lower_bit;
            let upper_mask =
                if upper_bit == WORD_BITS { usize::MAX } else { (1usize << upper_bit) - 1 };
            let candidates = self.words[word_index] & lower_mask & upper_mask;
            if candidates != 0 {
                return Some(word_start + candidates.trailing_zeros() as usize);
            }
        }

        None
    }
}
