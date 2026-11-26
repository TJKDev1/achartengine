//! Typed columns, the storage unit of a [`Dataset`](crate::Dataset).

use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec::Vec;

/// A named column of values, all of one type.
#[derive(Clone, Debug, PartialEq)]
pub struct Column {
    /// Name of the column, unique within its dataset.
    pub name: String,
    /// The values, one per row.
    pub values: ColumnValues,
}

impl Column {
    /// Creates a column from a name and its values.
    #[must_use]
    pub fn new(name: impl Into<String>, values: ColumnValues) -> Self {
        Self {
            name: name.into(),
            values,
        }
    }
}

/// The values of a column, stored as one contiguous array per type.
#[derive(Clone, Debug, PartialEq)]
pub enum ColumnValues {
    /// 64-bit floats, such as prices.
    F64(Vec<f64>),
    /// 64-bit signed integers, such as volumes or counts.
    I64(Vec<i64>),
    /// Points in time.
    Timestamp(Vec<Timestamp>),
    /// Strings, dictionary-encoded.
    Str(StrValues),
    /// Booleans.
    Bool(Vec<bool>),
}

impl ColumnValues {
    /// Number of values (rows) in the column.
    #[must_use]
    pub fn len(&self) -> usize {
        match self {
            Self::F64(values) => values.len(),
            Self::I64(values) => values.len(),
            Self::Timestamp(values) => values.len(),
            Self::Str(values) => values.len(),
            Self::Bool(values) => values.len(),
        }
    }

    /// Returns `true` if the column has no values.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The type of the values.
    #[must_use]
    pub fn column_type(&self) -> ColumnType {
        match self {
            Self::F64(_) => ColumnType::F64,
            Self::I64(_) => ColumnType::I64,
            Self::Timestamp(_) => ColumnType::Timestamp,
            Self::Str(_) => ColumnType::Str,
            Self::Bool(_) => ColumnType::Bool,
        }
    }
}

/// The type of a column's values, without the values themselves.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ColumnType {
    /// See [`ColumnValues::F64`].
    F64,
    /// See [`ColumnValues::I64`].
    I64,
    /// See [`ColumnValues::Timestamp`].
    Timestamp,
    /// See [`ColumnValues::Str`].
    Str,
    /// See [`ColumnValues::Bool`].
    Bool,
}

/// A point in time, in nanoseconds since the Unix epoch (UTC).
///
/// An `i64` of nanoseconds covers the years 1677 to 2262. Timestamps carry no time zone; a
/// frontend or calendar plugin converts them for display.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Timestamp(pub i64);

/// Dictionary-encoded strings: each distinct string is stored once, and each row holds the
/// index of its string.
///
/// Symbols, exchange names and categories repeat on most rows, so a 1M-row column of a few
/// hundred symbols costs 4 MB of codes plus the dictionary, instead of a `String` per row.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct StrValues {
    // Invariant: every code is a valid index into `dictionary`.
    dictionary: Vec<String>,
    codes: Vec<u32>,
}

impl StrValues {
    /// Encodes `strs`, giving each distinct string a code in order of first appearance.
    ///
    /// Returns `None` if there are more than `u32::MAX` distinct strings.
    #[must_use]
    pub fn from_strs<'a>(strs: impl IntoIterator<Item = &'a str>) -> Option<Self> {
        let mut code_of: BTreeMap<&str, u32> = BTreeMap::new();
        let mut encoded = Self::default();
        for s in strs {
            let code = if let Some(&code) = code_of.get(s) {
                code
            } else {
                // First time we see `s`: give it the next code.
                let code = u32::try_from(encoded.dictionary.len()).ok()?;
                code_of.insert(s, code);
                encoded.dictionary.push(String::from(s));
                code
            };
            encoded.codes.push(code);
        }
        Some(encoded)
    }

    /// The string in `row`, or `None` if `row` is out of range.
    #[must_use]
    pub fn get(&self, row: usize) -> Option<&str> {
        let code = *self.codes.get(row)?;
        self.dictionary
            .get(usize::try_from(code).ok()?)
            .map(String::as_str)
    }

    /// Number of rows.
    #[must_use]
    pub fn len(&self) -> usize {
        self.codes.len()
    }

    /// Returns `true` if there are no rows.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.codes.is_empty()
    }

    /// The distinct strings, indexed by code.
    #[must_use]
    pub fn dictionary(&self) -> &[String] {
        &self.dictionary
    }

    /// One code per row, each an index into [`dictionary`](Self::dictionary).
    #[must_use]
    pub fn codes(&self) -> &[u32] {
        &self.codes
    }
}

#[cfg(test)]
mod tests {
    use super::{ColumnType, ColumnValues, StrValues, Timestamp};
    use alloc::vec;

    #[test]
    fn len_and_type_match_each_variant() {
        let symbols = StrValues::from_strs(["BTC", "ETH"]).unwrap();
        let cases = [
            (ColumnValues::F64(vec![1.0, 2.0]), ColumnType::F64),
            (ColumnValues::I64(vec![1, 2]), ColumnType::I64),
            (
                ColumnValues::Timestamp(vec![Timestamp(1), Timestamp(2)]),
                ColumnType::Timestamp,
            ),
            (ColumnValues::Str(symbols), ColumnType::Str),
            (ColumnValues::Bool(vec![true, false]), ColumnType::Bool),
        ];
        for (values, column_type) in cases {
            assert_eq!(values.len(), 2, "{column_type:?} column has two rows");
            assert!(!values.is_empty(), "{column_type:?} column is not empty");
            assert_eq!(values.column_type(), column_type, "type of {values:?}");
        }
    }

    #[test]
    fn empty_column_is_empty() {
        let values = ColumnValues::F64(vec![]);
        assert!(values.is_empty(), "a column with no values is empty");
    }

    #[test]
    fn strings_are_encoded_once_in_order_of_first_appearance() {
        let symbols = StrValues::from_strs(["ETH", "BTC", "ETH", "ETH", "BTC"]).unwrap();
        assert_eq!(symbols.dictionary(), ["ETH", "BTC"], "dictionary");
        assert_eq!(symbols.codes(), [0, 1, 0, 0, 1], "codes");
        assert_eq!(symbols.len(), 5, "one code per row");
        assert!(!symbols.is_empty(), "five rows is not empty");
    }

    #[test]
    fn get_decodes_each_row() {
        let symbols = StrValues::from_strs(["ETH", "BTC", "ETH"]).unwrap();
        assert_eq!(symbols.get(0), Some("ETH"), "row 0");
        assert_eq!(symbols.get(1), Some("BTC"), "row 1");
        assert_eq!(symbols.get(2), Some("ETH"), "row 2");
        assert_eq!(symbols.get(3), None, "past the last row");
    }

    #[test]
    fn no_strings_give_an_empty_column() {
        let symbols = StrValues::from_strs([]).unwrap();
        assert!(symbols.is_empty(), "no rows");
        assert!(symbols.dictionary().is_empty(), "no dictionary entries");
    }
}
