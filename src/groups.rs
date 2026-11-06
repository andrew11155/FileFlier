//! Grouping the listed items, by name (first letter) or by date modified (Today,
//! Yesterday, This week, This month, This year, then each year), shown with a header
//! above each group. Which grouping a folder uses is remembered per folder.

use std::time::SystemTime;

use chrono::{Datelike, Local, NaiveDate};

use crate::config::GroupBy;
use crate::fs_model::Entry;

/// Orders `visible` (indices into `entries`) group by group, keeping their order within
/// each group, and returns where each group starts in it and its label.
pub fn group(by: GroupBy, entries: &[Entry], visible: &mut [usize]) -> Vec<(usize, String)> {
    if by == GroupBy::None || visible.is_empty() {
        return Vec::new();
    }
    let today = Local::now().date_naive();
    let keyed: Vec<(u64, String)> = visible.iter().map(|&i| key(by, &entries[i], today)).collect();
    let mut order: Vec<usize> = (0..visible.len()).collect();
    order.sort_by_key(|&k| keyed[k].0); // stable: the sort order stays within a group
    let sorted: Vec<usize> = order.iter().map(|&k| visible[k]).collect();
    visible.copy_from_slice(&sorted);
    let mut groups: Vec<(usize, String)> = Vec::new();
    let mut last: Option<u64> = None;
    for (at, &k) in order.iter().enumerate() {
        if last != Some(keyed[k].0) {
            last = Some(keyed[k].0);
            groups.push((at, keyed[k].1.clone()));
        }
    }
    groups
}

/// An item's group: (rank, label). Lower ranks come first.
fn key(by: GroupBy, e: &Entry, today: NaiveDate) -> (u64, String) {
    match by {
        GroupBy::None => (0, String::new()),
        GroupBy::Name => by_name(&e.name),
        GroupBy::Modified => by_date(e.modified, today),
    }
}

/// By first letter (a leading dot is skipped): symbols, then digits, then the letters.
fn by_name(name: &str) -> (u64, String) {
    let c = name.trim_start_matches('.').chars().next().unwrap_or('#');
    if c.is_alphabetic() {
        let lower = c.to_lowercase().next().unwrap_or(c);
        (100 + lower as u64, c.to_uppercase().collect())
    } else if c.is_ascii_digit() {
        (1, "0–9".into())
    } else {
        (0, "#".into())
    }
}

/// Newest first: Today, Yesterday, This week, This month, This year, then each year.
fn by_date(modified: Option<SystemTime>, today: NaiveDate) -> (u64, String) {
    let Some(t) = modified else { return (u64::MAX, "Unknown date".into()) };
    let d = chrono::DateTime::<Local>::from(t).date_naive();
    let days = (today - d).num_days();
    let week_start = today - chrono::Duration::days(today.weekday().num_days_from_monday() as i64);
    if days <= 0 {
        (0, "Today".into()) // (or later, with a clock that was ahead)
    } else if days == 1 {
        (1, "Yesterday".into())
    } else if d >= week_start {
        (2, "This week".into())
    } else if d.year() == today.year() && d.month() == today.month() {
        (3, "This month".into())
    } else if d.year() == today.year() {
        (4, "This year".into())
    } else {
        (10 + (today.year() - d.year()).max(0) as u64, d.year().to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn at(y: i32, m: u32, d: u32) -> Option<SystemTime> {
        let dt = NaiveDate::from_ymd_opt(y, m, d)?.and_hms_opt(12, 0, 0)?.and_local_timezone(Local).single()?;
        Some(dt.into())
    }

    #[test]
    fn dates_fall_into_buckets() {
        // Thursday 2026-11-05.
        let today = NaiveDate::from_ymd_opt(2026, 11, 5).unwrap();
        let label = |t| by_date(t, today).1;
        assert_eq!(label(at(2026, 11, 5)), "Today");
        assert_eq!(label(at(2026, 11, 4)), "Yesterday");
        assert_eq!(label(at(2026, 11, 2)), "This week"); // Monday
        assert_eq!(label(at(2026, 11, 1)), "This month"); // the Sunday before
        assert_eq!(label(at(2026, 3, 9)), "This year");
        assert_eq!(label(at(2024, 12, 31)), "2024");
        assert_eq!(label(None), "Unknown date");
        assert!(by_date(at(2025, 1, 1), today).0 < by_date(at(2024, 1, 1), today).0);
    }

    #[test]
    fn names_group_by_first_letter() {
        assert_eq!(by_name("apple").1, "A");
        assert_eq!(by_name("Apple").0, by_name("apple").0);
        assert_eq!(by_name(".bashrc").1, "B");
        assert_eq!(by_name("2024 taxes").1, "0–9");
        assert_eq!(by_name("_notes").1, "#");
        assert_eq!(by_name("über").1, "Ü");
        assert!(by_name("_x").0 < by_name("1x").0 && by_name("1x").0 < by_name("ax").0);
    }

    #[test]
    fn grouping_keeps_the_order_within_groups() {
        let e = |name: &str| Entry {
            name: name.into(),
            path: PathBuf::from(name),
            is_dir: false,
            is_symlink: false,
            is_file: true,
            size: 0,
            modified: None,
        };
        let entries = vec![e("bee"), e("ant"), e("bat"), e("apple")];
        let mut visible = vec![0, 1, 2, 3];
        let groups = group(GroupBy::Name, &entries, &mut visible);
        assert_eq!(visible, vec![1, 3, 0, 2]); // ant, apple | bee, bat
        assert_eq!(groups, vec![(0, "A".to_string()), (2, "B".to_string())]);
        let mut visible = vec![0, 1];
        assert!(group(GroupBy::None, &entries, &mut visible).is_empty());
    }
}
