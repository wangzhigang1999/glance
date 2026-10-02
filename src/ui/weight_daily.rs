//! Calendar-day summaries over retained local measurements, never remote history.
use std::collections::BTreeMap;

pub struct Day {
    pub day: i64,
    pub median: f32,
    pub min: f32,
    pub max: f32,
}

pub fn summarize(readings: impl IntoIterator<Item = (Option<i64>, f32)>, offset: i64) -> Vec<Day> {
    let mut groups: BTreeMap<i64, Vec<f32>> = BTreeMap::new();
    for (timestamp, kg) in readings {
        if let Some(time) = timestamp.filter(|_| kg.is_finite()) {
            groups
                .entry((time + offset).div_euclid(86400))
                .or_default()
                .push(kg);
        }
    }
    groups
        .into_iter()
        .map(|(day, mut values)| {
            values.sort_by(f32::total_cmp);
            let middle = values.len() / 2;
            let median = if values.len() % 2 == 0 {
                (values[middle - 1] + values[middle]) / 2.0
            } else {
                values[middle]
            };
            Day {
                day,
                median,
                min: values[0],
                max: *values.last().unwrap(),
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn local_midnight_medians_and_unknown_timestamps() {
        let result = summarize(
            [
                (Some(57599), 55.0),
                (Some(57600), 58.0),
                (Some(57601), 56.0),
                (None, 90.0),
            ],
            8 * 3600,
        );
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].day, 0);
        assert_eq!(result[1].day, 1);
        assert_eq!(result[1].median, 57.0);
        assert_eq!(result[1].min, 56.0);
        assert_eq!(result[1].max, 58.0);
    }
    #[test]
    fn empty_and_odd_samples() {
        assert!(summarize([], 0).is_empty());
        let days = summarize([(Some(0), 58.0), (Some(1), 55.0), (Some(2), 56.0)], 0);
        assert_eq!(days[0].median, 56.0);
    }
}
