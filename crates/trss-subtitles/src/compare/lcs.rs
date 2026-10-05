//! A longest common subsequence of two sequences, in linear memory
//! (Hirschberg), so a file of thousands of lines costs no table of their
//! product.

/// The index pairs `(i, j)` with `a[i] == b[j]` of one longest common
/// subsequence, ascending in both.
pub fn align(a: &[u32], b: &[u32]) -> Vec<(usize, usize)> {
    let prefix = a.iter().zip(b).take_while(|(x, y)| x == y).count();
    let suffix = a[prefix..]
        .iter()
        .rev()
        .zip(b[prefix..].iter().rev())
        .take_while(|(x, y)| x == y)
        .count();
    let (a_mid, b_mid) = (&a[prefix..a.len() - suffix], &b[prefix..b.len() - suffix]);
    let mut pairs: Vec<(usize, usize)> = (0..prefix).map(|i| (i, i)).collect();
    hirschberg(a_mid, b_mid, prefix, prefix, &mut pairs);
    pairs.extend((0..suffix).map(|k| (a.len() - suffix + k, b.len() - suffix + k)));
    pairs
}

fn hirschberg(a: &[u32], b: &[u32], a_at: usize, b_at: usize, out: &mut Vec<(usize, usize)>) {
    if a.is_empty() || b.is_empty() {
        return;
    }
    if a.len() == 1 {
        if let Some(j) = b.iter().position(|x| *x == a[0]) {
            out.push((a_at, b_at + j));
        }
        return;
    }
    let mid = a.len() / 2;
    let head = lengths(a[..mid].iter(), b.iter());
    let tail = lengths(a[mid..].iter().rev(), b.iter().rev());
    let split = (0..=b.len())
        .max_by_key(|&j| (head[j] + tail[b.len() - j], std::cmp::Reverse(j)))
        .unwrap_or(0);
    hirschberg(&a[..mid], &b[..split], a_at, b_at, out);
    hirschberg(&a[mid..], &b[split..], a_at + mid, b_at + split, out);
}

/// `row[k]`: the longest common subsequence length of all of `a` and the
/// first `k` of `b`.
fn lengths<'a>(a: impl Iterator<Item = &'a u32>, b: impl Iterator<Item = &'a u32>) -> Vec<usize> {
    let b: Vec<&u32> = b.collect();
    let mut row = vec![0usize; b.len() + 1];
    for x in a {
        let mut diagonal = 0;
        for (k, y) in b.iter().enumerate() {
            let above = row[k + 1];
            row[k + 1] = match x == *y {
                true => diagonal + 1,
                false => row[k + 1].max(row[k]),
            };
            diagonal = above;
        }
    }
    row
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The textbook table, for small inputs.
    fn length(a: &[u32], b: &[u32]) -> usize {
        let mut table = vec![vec![0; b.len() + 1]; a.len() + 1];
        for i in 0..a.len() {
            for j in 0..b.len() {
                table[i + 1][j + 1] = match a[i] == b[j] {
                    true => table[i][j] + 1,
                    false => table[i][j + 1].max(table[i + 1][j]),
                };
            }
        }
        table[a.len()][b.len()]
    }

    #[test]
    fn matches_the_textbook_length_and_is_a_valid_alignment() {
        // A small deterministic generator; no dependency for it.
        let mut state = 0x2545_F491_4F6C_DD1Du64;
        let mut next = move |modulus: u64| {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            (state % modulus) as u32
        };
        for _ in 0..300 {
            let a: Vec<u32> = (0..next(30)).map(|_| next(5)).collect();
            let b: Vec<u32> = (0..next(30)).map(|_| next(5)).collect();
            let pairs = align(&a, &b);
            assert_eq!(pairs.len(), length(&a, &b), "{a:?} {b:?}");
            assert!(pairs.iter().all(|&(i, j)| a[i] == b[j]));
            assert!(pairs.windows(2).all(|w| w[0].0 < w[1].0 && w[0].1 < w[1].1));
        }
    }

    #[test]
    fn empty_and_identical_inputs() {
        assert!(align(&[], &[1, 2]).is_empty());
        assert!(align(&[1], &[]).is_empty());
        assert_eq!(align(&[1, 2, 3], &[1, 2, 3]), vec![(0, 0), (1, 1), (2, 2)]);
    }
}
