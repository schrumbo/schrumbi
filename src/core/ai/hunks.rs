use std::ops::Range;

/// One contiguous change between two texts, as line ranges counted from 0.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hunk {
    pub old: Range<usize>,
    pub new: Range<usize>,
}

fn closed(text: &str) -> String {
    if text.is_empty() || text.ends_with('\n') { text.to_string() } else { format!("{text}\n") }
}

/// The changes from `before` to `after`, in order; lines that only differ by a missing final line break count as equal.
pub fn hunks(before: &str, after: &str) -> Vec<Hunk> {
    let (old, new) = (closed(before), closed(after));
    let diff = similar::TextDiff::from_lines(&old, &new);
    let mut out: Vec<Hunk> = Vec::new();
    let mut open = false;
    for op in diff.ops() {
        if op.tag() == similar::DiffTag::Equal {
            open = false;
            continue;
        }
        match out.last_mut() {
            Some(last) if open => {
                last.old.end = op.old_range().end;
                last.new.end = op.new_range().end;
            }
            _ => out.push(Hunk { old: op.old_range(), new: op.new_range() }),
        }
        open = true;
    }
    out
}

/// `after` with the changes whose `keep` is false taken back to `before`; unlisted changes are kept.
pub fn merge(before: &str, after: &str, keep: &[bool]) -> String {
    let (old, new) = (closed(before), closed(after));
    let old_lines: Vec<&str> = old.split_inclusive('\n').collect();
    let new_lines: Vec<&str> = new.split_inclusive('\n').collect();
    let mut out = String::new();
    let mut last: (Option<usize>, Option<usize>) = (None, None);
    let (mut at, mut old_at) = (0, 0);
    let mut equal = |out: &mut String, last: &mut (Option<usize>, Option<usize>), to: usize, at: usize, old_at: usize| {
        for (k, line) in new_lines[at..to].iter().enumerate() {
            out.push_str(line);
            *last = (Some(old_at + k), Some(at + k));
        }
    };
    for (i, h) in hunks(before, after).iter().enumerate() {
        equal(&mut out, &mut last, h.new.start, at, old_at);
        if keep.get(i).copied().unwrap_or(true) {
            for j in h.new.clone() {
                out.push_str(new_lines[j]);
                last = (None, Some(j));
            }
        } else {
            for j in h.old.clone() {
                out.push_str(old_lines[j]);
                last = (Some(j), None);
            }
        }
        at = h.new.end;
        old_at = h.old.end;
    }
    equal(&mut out, &mut last, new_lines.len(), at, old_at);
    let open = |text: &str| !text.is_empty() && !text.ends_with('\n');
    let new_last = last.1.is_some_and(|n| n + 1 == new_lines.len());
    let old_last = last.0.is_some_and(|o| o + 1 == old_lines.len());
    if ((new_last && open(after)) || (!new_last && old_last && open(before))) && out.ends_with('\n') {
        out.pop();
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_each_change() {
        let h = hunks("a\nb\nc\nd\n", "a\nB\nc\nd\ne\n");
        assert_eq!(h, vec![Hunk { old: 1..2, new: 1..2 }, Hunk { old: 4..4, new: 4..5 }]);
    }

    #[test]
    fn keeping_all_or_none_gives_either_side() {
        let (b, a) = ("eins\nzwei\ndrei", "eins\nZWEI\ndrei\nvier");
        assert_eq!(merge(b, a, &[true, true]), a);
        assert_eq!(merge(b, a, &[false, false]), b);
    }

    #[test]
    fn takes_back_only_the_rejected_change() {
        let (b, a) = ("a\nb\nc\nd\n", "a\nB\nc\nd\ne\n");
        assert_eq!(merge(b, a, &[false, true]), "a\nb\nc\nd\ne\n");
        assert_eq!(merge(b, a, &[true, false]), "a\nB\nc\nd\n");
    }

    #[test]
    fn keeps_line_endings_and_umlauts() {
        let (b, a) = ("Äpfel\r\nBirnen\r\n", "Äpfel\r\nBörnen\r\nÜbung\r\n");
        assert_eq!(merge(b, a, &[false]), b);
        assert_eq!(hunks(b, a).len(), 1);
    }

    #[test]
    fn final_line_break_follows_the_side_of_the_last_line() {
        assert_eq!(merge("x\n", "x\ny", &[false]), "x\n");
        assert_eq!(merge("x", "x\ny\n", &[false]), "x");
        assert_eq!(merge("x\ny", "x\nz\n", &[false]), "x\ny");
        assert_eq!(merge("x\ny", "x\nz\n", &[true]), "x\nz\n");
    }

    #[test]
    fn handles_empty_sides() {
        assert_eq!(merge("", "neu\n", &[false]), "");
        assert_eq!(merge("alt\n", "", &[false]), "alt\n");
        assert_eq!(merge("alt", "", &[true]), "");
    }
}
