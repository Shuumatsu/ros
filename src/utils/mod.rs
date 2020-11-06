pub fn extract_value(bits: usize, mask: usize, start_pos: usize) -> usize {
    (bits & (mask << start_pos)) >> start_pos
}
