use std::ops::Range;

/// The rows as lines of columns two spaces apart, the columns in `right` aligned to the right, without spaces at the end.
pub(crate) fn table(rows: &[Vec<String>], right: Range<usize>) -> Vec<String> {
    let mut widths: Vec<usize> = Vec::new();
    for row in rows {
        for (column, cell) in row.iter().enumerate() {
            let width = cell.chars().count();
            match widths.get_mut(column) {
                Some(widest) => *widest = (*widest).max(width),
                None => widths.push(width),
            }
        }
    }
    rows.iter()
        .map(|row| {
            let cells: Vec<String> = row
                .iter()
                .zip(&widths)
                .enumerate()
                .map(|(column, (cell, &width))| {
                    if right.contains(&column) {
                        format!("{cell:>width$}")
                    } else if column + 1 == row.len() {
                        cell.clone()
                    } else {
                        format!("{cell:width$}")
                    }
                })
                .collect();
            cells.join("  ")
        })
        .collect()
}

/// A count of tokens as people read it, such as `950`, `34.2k` or `1.5M`.
pub(crate) fn tokens(count: u64) -> String {
    match count {
        ..1000 => count.to_string(),
        1000..999_950 => {
            let tenths = (count + 50) / 100;
            format!("{}.{}k", tenths / 10, tenths % 10)
        }
        _ => {
            let tenths = (count + 50_000) / 100_000;
            format!("{}.{}M", tenths / 10, tenths % 10)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{table, tokens};

    #[test]
    fn tokens_read_as_a_count_then_thousands_then_millions() {
        let shown = [
            0, 999, 1000, 34_249, 34_250, 999_949, 999_950, 1_449_999, 12_345_678,
        ]
        .map(tokens);

        assert_eq!(
            shown,
            [
                "0", "999", "1.0k", "34.2k", "34.3k", "999.9k", "1.0M", "1.4M", "12.3M"
            ]
        );
    }

    #[test]
    fn a_table_pads_its_columns_aligns_the_right_ones_and_ends_each_line_with_its_last_cell() {
        let rows = [
            vec!["CT-0001", "done", "2 in", "unsandboxed"],
            vec!["CT-0002", "working", "1.2k in"],
        ]
        .map(|row| row.into_iter().map(str::to_owned).collect::<Vec<String>>());

        let lines = table(&rows, 2..3);

        assert_eq!(
            lines,
            [
                "CT-0001  done        2 in  unsandboxed",
                "CT-0002  working  1.2k in",
            ]
        );
    }
}
