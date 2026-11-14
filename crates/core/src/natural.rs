//! Natural, case-insensitive, Polish-aware file name ordering
//! (`file2` < `file10`, `a` < `ą` < `b`, `z` < `ź` < `ż`), close to
//! Explorer's behaviour under a Polish locale.

use std::cmp::Ordering;

/// Sort weight of a char. Polish letters are distinct letters placed right
/// after their base letter (`base * 4 + n`), as in the Polish alphabet.
#[inline]
fn weight(c: char) -> u32 {
    let (base, n) = match c {
        'ą' | 'Ą' => ('a', 1),
        'ć' | 'Ć' => ('c', 1),
        'ę' | 'Ę' => ('e', 1),
        'ł' | 'Ł' => ('l', 1),
        'ń' | 'Ń' => ('n', 1),
        'ó' | 'Ó' => ('o', 1),
        'ś' | 'Ś' => ('s', 1),
        'ź' | 'Ź' => ('z', 1),
        'ż' | 'Ż' => ('z', 2),
        c if c.is_ascii() => (c.to_ascii_lowercase(), 0),
        c => (c.to_lowercase().next().unwrap_or(c), 0),
    };
    base as u32 * 4 + n
}

pub fn natural_cmp(a: &str, b: &str) -> Ordering {
    let (ab, bb) = (a.as_bytes(), b.as_bytes());
    let (mut i, mut j) = (0usize, 0usize);

    while i < ab.len() && j < bb.len() {
        if ab[i].is_ascii_digit() && bb[j].is_ascii_digit() {
            let (si, sj) = (i, j);
            while i < ab.len() && ab[i].is_ascii_digit() {
                i += 1;
            }
            while j < bb.len() && bb[j].is_ascii_digit() {
                j += 1;
            }
            let da = trim_zeros(&ab[si..i]);
            let db = trim_zeros(&bb[sj..j]);
            let ord = da.len().cmp(&db.len()).then_with(|| da.cmp(db));
            if ord != Ordering::Equal {
                return ord;
            }
            continue;
        }
        // i/j always sit on char boundaries: we only advance by whole chars
        // or by ASCII digit runs.
        let ca = a[i..].chars().next().unwrap();
        let cb = b[j..].chars().next().unwrap();
        i += ca.len_utf8();
        j += cb.len_utf8();
        let ord = weight(ca).cmp(&weight(cb));
        if ord != Ordering::Equal {
            return ord;
        }
    }
    (ab.len() - i)
        .cmp(&(bb.len() - j))
        // Fully deterministic: "f01" vs "f1", "A" vs "a".
        .then_with(|| a.cmp(b))
}

fn trim_zeros(d: &[u8]) -> &[u8] {
    let k = d.iter().position(|&c| c != b'0').unwrap_or(d.len());
    &d[k..]
}

/// rank[i] = position of names[i] in natural order.
pub fn name_ranks(names: &[String]) -> Vec<u32> {
    let mut idx: Vec<u32> = (0..names.len() as u32).collect();
    idx.sort_unstable_by(|&x, &y| natural_cmp(&names[x as usize], &names[y as usize]));
    let mut rank = vec![0u32; names.len()];
    for (pos, &i) in idx.iter().enumerate() {
        rank[i as usize] = pos as u32;
    }
    rank
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sorted(v: &[&str]) -> Vec<String> {
        let mut v: Vec<String> = v.iter().map(|s| s.to_string()).collect();
        v.sort_by(|a, b| natural_cmp(a, b));
        v
    }

    #[test]
    fn numbers_are_numeric() {
        assert_eq!(sorted(&["f10", "f2", "f1", "f02"]), ["f1", "f02", "f2", "f10"]);
    }

    #[test]
    fn case_insensitive() {
        assert_eq!(sorted(&["b", "a", "B", "A"]), ["A", "a", "B", "b"]);
    }

    #[test]
    fn polish_letters() {
        assert_eq!(
            sorted(&["zebra", "żaba", "źdźbło", "ala", "ąbc", "bob", "Łódź", "lis"]),
            ["ala", "ąbc", "bob", "lis", "Łódź", "zebra", "źdźbło", "żaba"]
        );
    }

    #[test]
    fn prefix_first() {
        assert_eq!(sorted(&["abc", "ab", "abcd"]), ["ab", "abc", "abcd"]);
    }

    #[test]
    fn ranks() {
        let names: Vec<String> = ["c", "a", "b"].iter().map(|s| s.to_string()).collect();
        assert_eq!(name_ranks(&names), [2, 0, 1]);
    }
}
