//! Datasets: named columns of equal length, optionally ordered by a key column (DESIGN §5).

use alloc::string::String;
use alloc::vec::Vec;
use core::fmt;

use crate::column::{Column, ColumnType, ColumnValues};

/// A set of named columns of equal length, optionally ordered by a key column.
///
/// Bars are a dataset with `t, open, high, low, close, volume` columns keyed by `t`. Nothing here
/// assumes time, though: a volume profile is keyed by price.
///
/// The checks in [`new`](Self::new) hold for the dataset's whole life, so code that reads a
/// dataset can rely on them without checking again.
#[derive(Clone, Debug, PartialEq)]
pub struct Dataset {
    columns: Vec<Column>,
    key: Option<usize>,
    row_count: usize,
}

impl Dataset {
    /// Creates a dataset from `columns`, ordered by the column named `key` if there is one.
    ///
    /// A key must be an `f64`, `i64` or timestamp column, sorted ascending. Equal neighbours are
    /// allowed, because ticks can share a timestamp. An `f64` key must not contain NaN.
    ///
    /// # Errors
    ///
    /// Returns the first of these that applies: two columns share a name, the columns differ
    /// in length, `key` names no column, the key column has the wrong type, contains NaN, or
    /// is out of order. The columns are dropped with the error.
    pub fn new(columns: Vec<Column>, key: Option<&str>) -> Result<Self, DatasetError> {
        check_unique_names(&columns)?;
        let row_count = check_equal_lengths(&columns)?;
        let key = match key {
            Some(name) => Some(find_sorted_key(&columns, name)?),
            None => None,
        };
        Ok(Self {
            columns,
            key,
            row_count,
        })
    }

    /// Number of rows, which every column shares.
    #[must_use]
    pub fn row_count(&self) -> usize {
        self.row_count
    }

    /// All columns, in the order they were given.
    #[must_use]
    pub fn columns(&self) -> &[Column] {
        &self.columns
    }

    /// The column called `name`, if there is one.
    #[must_use]
    pub fn column(&self, name: &str) -> Option<&Column> {
        self.columns.iter().find(|column| column.name == name)
    }

    /// The key column, if the dataset has one.
    #[must_use]
    pub fn key(&self) -> Option<&Column> {
        self.columns.get(self.key?)
    }
}

/// Why [`Dataset::new`] rejected its columns.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DatasetError {
    /// Two columns have the same name.
    DuplicateName {
        /// The repeated name.
        column: String,
    },
    /// A column's length differs from the first column's.
    LengthMismatch {
        /// The column with the different length.
        column: String,
        /// Length of the first column.
        expected: usize,
        /// Length of this column.
        found: usize,
    },
    /// No column has the name given as the key.
    KeyNotFound {
        /// The name given as the key.
        column: String,
    },
    /// The key column's type has no order to sort by.
    KeyTypeNotSortable {
        /// The key column.
        column: String,
        /// Its type.
        column_type: ColumnType,
    },
    /// The key column is not sorted ascending.
    KeyNotSorted {
        /// The key column.
        column: String,
        /// The first row that is smaller than the row before it.
        row: usize,
    },
    /// The `f64` key column contains NaN, which has no place in an order.
    KeyContainsNan {
        /// The key column.
        column: String,
        /// The first row that is NaN.
        row: usize,
    },
}

impl fmt::Display for DatasetError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DuplicateName { column } => write!(f, "two columns are named `{column}`"),
            Self::LengthMismatch {
                column,
                expected,
                found,
            } => write!(
                f,
                "column `{column}` has {found} rows, but the first column has {expected}"
            ),
            Self::KeyNotFound { column } => write!(f, "no column named `{column}` for the key"),
            Self::KeyTypeNotSortable {
                column,
                column_type,
            } => write!(
                f,
                "key column `{column}` has type {column_type:?}, which can't be sorted"
            ),
            Self::KeyNotSorted { column, row } => {
                write!(f, "key column `{column}` is out of order at row {row}")
            }
            Self::KeyContainsNan { column, row } => {
                write!(f, "key column `{column}` is NaN at row {row}")
            }
        }
    }
}

impl core::error::Error for DatasetError {}

fn check_unique_names(columns: &[Column]) -> Result<(), DatasetError> {
    // Datasets have a handful of columns, so comparing every pair is cheaper than a set.
    for (i, column) in columns.iter().enumerate() {
        if columns
            .iter()
            .skip(i + 1)
            .any(|other| other.name == column.name)
        {
            return Err(DatasetError::DuplicateName {
                column: column.name.clone(),
            });
        }
    }
    Ok(())
}

/// Returns the shared row count.
fn check_equal_lengths(columns: &[Column]) -> Result<usize, DatasetError> {
    let Some(first) = columns.first() else {
        return Ok(0);
    };
    let expected = first.values.len();
    match columns
        .iter()
        .find(|column| column.values.len() != expected)
    {
        Some(column) => Err(DatasetError::LengthMismatch {
            column: column.name.clone(),
            expected,
            found: column.values.len(),
        }),
        None => Ok(expected),
    }
}

/// Returns the index of the key column after checking that it can be and is sorted.
fn find_sorted_key(columns: &[Column], name: &str) -> Result<usize, DatasetError> {
    let Some((index, column)) = columns.iter().enumerate().find(|(_, c)| c.name == name) else {
        return Err(DatasetError::KeyNotFound {
            column: String::from(name),
        });
    };
    match &column.values {
        ColumnValues::F64(values) => {
            check_no_nan(column, values)?;
            check_sorted(column, values)?;
        }
        ColumnValues::I64(values) => check_sorted(column, values)?,
        ColumnValues::Timestamp(values) => check_sorted(column, values)?,
        ColumnValues::Str(_) | ColumnValues::Bool(_) => {
            return Err(DatasetError::KeyTypeNotSortable {
                column: column.name.clone(),
                column_type: column.values.column_type(),
            });
        }
    }
    Ok(index)
}

fn check_no_nan(column: &Column, values: &[f64]) -> Result<(), DatasetError> {
    match values.iter().position(|value| value.is_nan()) {
        Some(row) => Err(DatasetError::KeyContainsNan {
            column: column.name.clone(),
            row,
        }),
        None => Ok(()),
    }
}

/// Checks that `values` never decrease. NaN must already be ruled out, since it compares false.
fn check_sorted<T: PartialOrd>(column: &Column, values: &[T]) -> Result<(), DatasetError> {
    let mut pairs = values.iter().zip(values.iter().skip(1));
    match pairs.position(|(previous, next)| previous > next) {
        Some(pair) => Err(DatasetError::KeyNotSorted {
            column: column.name.clone(),
            row: pair + 1,
        }),
        None => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::{Dataset, DatasetError};
    use crate::column::{Column, ColumnType, ColumnValues, StrValues, Timestamp};
    use alloc::borrow::ToOwned;
    use alloc::string::ToString;
    use alloc::vec;
    use alloc::vec::Vec;

    fn times(nanos: &[i64]) -> Column {
        Column::new(
            "t",
            ColumnValues::Timestamp(nanos.iter().copied().map(Timestamp).collect()),
        )
    }

    fn prices(name: &str, values: &[f64]) -> Column {
        Column::new(name, ColumnValues::F64(values.to_vec()))
    }

    #[test]
    fn bars_keyed_by_time() {
        let bars = Dataset::new(
            vec![times(&[1, 2, 3]), prices("close", &[10.0, 11.0, 10.5])],
            Some("t"),
        )
        .unwrap();
        assert_eq!(bars.row_count(), 3, "row count");
        assert_eq!(bars.columns().len(), 2, "column count");
        assert_eq!(bars.key().map(|c| c.name.as_str()), Some("t"), "key");
        assert_eq!(
            bars.column("close").map(|c| &c.values),
            Some(&ColumnValues::F64(vec![10.0, 11.0, 10.5])),
            "close column"
        );
        assert_eq!(bars.column("open"), None, "missing column");
    }

    #[test]
    fn key_can_be_a_later_column() {
        let profile = Dataset::new(
            vec![
                Column::new("volume", ColumnValues::I64(vec![5, 9])),
                prices("price", &[99.5, 100.0]),
            ],
            Some("price"),
        )
        .unwrap();
        assert_eq!(
            profile.key().map(|c| c.name.as_str()),
            Some("price"),
            "key is the second column"
        );
    }

    #[test]
    fn dataset_without_key() {
        let dataset = Dataset::new(vec![prices("close", &[2.0, 1.0])], None).unwrap();
        assert_eq!(dataset.key(), None, "no key");
        assert_eq!(
            dataset.row_count(),
            2,
            "unsorted values are fine without a key"
        );
    }

    #[test]
    fn dataset_without_columns_has_no_rows() {
        let dataset = Dataset::new(Vec::new(), None).unwrap();
        assert_eq!(dataset.row_count(), 0, "row count");
    }

    #[test]
    fn equal_keys_are_sorted() {
        let ticks = Dataset::new(vec![times(&[1, 1, 2])], Some("t"));
        assert!(ticks.is_ok(), "ticks may share a timestamp: {ticks:?}");
    }

    #[test]
    fn duplicate_names_are_rejected() {
        let result = Dataset::new(vec![prices("close", &[1.0]), prices("close", &[2.0])], None);
        assert_eq!(
            result,
            Err(DatasetError::DuplicateName {
                column: "close".to_owned()
            }),
            "duplicate"
        );
    }

    #[test]
    fn different_lengths_are_rejected() {
        let result = Dataset::new(vec![times(&[1, 2]), prices("close", &[1.0])], None);
        assert_eq!(
            result,
            Err(DatasetError::LengthMismatch {
                column: "close".to_owned(),
                expected: 2,
                found: 1
            }),
            "length mismatch"
        );
    }

    #[test]
    fn missing_key_is_rejected() {
        let result = Dataset::new(vec![times(&[1])], Some("time"));
        assert_eq!(
            result,
            Err(DatasetError::KeyNotFound {
                column: "time".to_owned()
            }),
            "missing key"
        );
    }

    #[test]
    fn string_and_bool_keys_are_rejected() {
        let symbols = StrValues::from_strs(["A", "B"]).unwrap();
        let cases = [
            (ColumnValues::Str(symbols), ColumnType::Str),
            (ColumnValues::Bool(vec![false, true]), ColumnType::Bool),
        ];
        for (values, column_type) in cases {
            let result = Dataset::new(vec![Column::new("k", values)], Some("k"));
            assert_eq!(
                result,
                Err(DatasetError::KeyTypeNotSortable {
                    column: "k".to_owned(),
                    column_type
                }),
                "{column_type:?} key"
            );
        }
    }

    #[test]
    fn unsorted_key_reports_the_first_row_out_of_order() {
        let result = Dataset::new(vec![times(&[1, 3, 2, 0])], Some("t"));
        assert_eq!(
            result,
            Err(DatasetError::KeyNotSorted {
                column: "t".to_owned(),
                row: 2
            }),
            "row 2 is smaller than row 1"
        );
    }

    #[test]
    fn unsorted_integer_key_is_rejected() {
        let column = Column::new("n", ColumnValues::I64(vec![2, 1]));
        let result = Dataset::new(vec![column], Some("n"));
        assert_eq!(
            result,
            Err(DatasetError::KeyNotSorted {
                column: "n".to_owned(),
                row: 1
            }),
            "i64 key"
        );
    }

    #[test]
    fn nan_in_a_float_key_is_rejected() {
        for (values, row) in [
            (&[f64::NAN, 1.0][..], 0),
            (&[1.0, f64::NAN][..], 1),
            (&[1.0, 2.0, f64::NAN][..], 2),
            (&[f64::NAN][..], 0),
            (&[3.0, 1.0, f64::NAN][..], 2),
        ] {
            let result = Dataset::new(vec![prices("price", values)], Some("price"));
            assert_eq!(
                result,
                Err(DatasetError::KeyContainsNan {
                    column: "price".to_owned(),
                    row
                }),
                "NaN in {values:?}"
            );
        }
    }

    #[test]
    fn the_first_failed_check_is_reported() {
        // Duplicate names, unequal lengths and an unsorted key at once: names are checked first.
        let columns = vec![times(&[2, 1]), times(&[1]), prices("close", &[1.0, 2.0])];
        assert!(
            matches!(
                Dataset::new(columns, Some("t")),
                Err(DatasetError::DuplicateName { .. })
            ),
            "names before lengths"
        );
        let columns = vec![times(&[2, 1]), prices("close", &[1.0])];
        assert!(
            matches!(
                Dataset::new(columns, Some("t")),
                Err(DatasetError::LengthMismatch { .. })
            ),
            "lengths before the key"
        );
    }

    #[test]
    fn errors_name_the_column() {
        let cases = [
            (
                DatasetError::DuplicateName {
                    column: "close".to_owned(),
                },
                "two columns are named `close`",
            ),
            (
                DatasetError::LengthMismatch {
                    column: "close".to_owned(),
                    expected: 3,
                    found: 2,
                },
                "column `close` has 2 rows, but the first column has 3",
            ),
            (
                DatasetError::KeyNotFound {
                    column: "t".to_owned(),
                },
                "no column named `t` for the key",
            ),
            (
                DatasetError::KeyTypeNotSortable {
                    column: "t".to_owned(),
                    column_type: ColumnType::Bool,
                },
                "key column `t` has type Bool, which can't be sorted",
            ),
            (
                DatasetError::KeyNotSorted {
                    column: "t".to_owned(),
                    row: 4,
                },
                "key column `t` is out of order at row 4",
            ),
            (
                DatasetError::KeyContainsNan {
                    column: "p".to_owned(),
                    row: 1,
                },
                "key column `p` is NaN at row 1",
            ),
        ];
        for (error, message) in cases {
            assert_eq!(error.to_string(), message, "message for {error:?}");
        }
    }
}
