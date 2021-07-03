use std::ops;

pub mod line_buffer;
pub mod lines;

/// The type of a match.
///
/// The type of a match is a possibly empty range pointing to a contiguous
/// block of addressable memory.
///
/// Every `Match` is guaranteed to satisfy the invariant that `start <= end`.
///
/// # Indexing
///
/// This type is structurally identical to `std::ops::Range<usize>`, but
/// is a bit more ergonomic for dealing with match indices. In particular,
/// this type implements `Copy` and provides methods for building new `Match`
/// values based on old `Match` values. Finally, the invariant that `start`
/// is always less than or equal to `end` is enforced.
///
/// A `Match` can be used to slice a `&[u8]`, `&mut [u8]` or `&str` using
/// range notation. e.g.,
///
/// ```
/// use ripline::Match;
///
/// let m = Match::new(2, 5);
/// let bytes = b"abcdefghi";
/// assert_eq!(b"cde", &bytes[m]);
/// ```
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct Match {
    start: usize,
    end: usize,
}

impl Match {
    /// Create a new match.
    ///
    /// # Panics
    ///
    /// This function panics if `start > end`.
    #[inline]
    pub fn new(start: usize, end: usize) -> Match {
        assert!(start <= end);
        Match { start, end }
    }

    /// Creates a zero width match at the given offset.
    #[inline]
    pub fn zero(offset: usize) -> Match {
        Match {
            start: offset,
            end: offset,
        }
    }

    /// Return the start offset of this match.
    #[inline]
    pub fn start(&self) -> usize {
        self.start
    }

    /// Return the end offset of this match.
    #[inline]
    pub fn end(&self) -> usize {
        self.end
    }

    /// Return a new match with the start offset replaced with the given
    /// value.
    ///
    /// # Panics
    ///
    /// This method panics if `start > self.end`.
    #[inline]
    pub fn with_start(&self, start: usize) -> Match {
        assert!(start <= self.end);
        Match { start, ..*self }
    }

    /// Return a new match with the end offset replaced with the given
    /// value.
    ///
    /// # Panics
    ///
    /// This method panics if `self.start > end`.
    #[inline]
    pub fn with_end(&self, end: usize) -> Match {
        assert!(self.start <= end);
        Match { end, ..*self }
    }

    /// Offset this match by the given amount and return a new match.
    ///
    /// This adds the given offset to the start and end of this match, and
    /// returns the resulting match.
    ///
    /// # Panics
    ///
    /// This panics if adding the given amount to either the start or end
    /// offset would result in an overflow.
    #[inline]
    pub fn offset(&self, amount: usize) -> Match {
        Match {
            start: self.start.checked_add(amount).unwrap(),
            end: self.end.checked_add(amount).unwrap(),
        }
    }

    /// Returns the number of bytes in this match.
    #[inline]
    pub fn len(&self) -> usize {
        self.end - self.start
    }

    /// Returns true if and only if this match is empty.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl ops::Index<Match> for [u8] {
    type Output = [u8];

    #[inline]
    fn index(&self, index: Match) -> &[u8] {
        &self[index.start..index.end]
    }
}

impl ops::IndexMut<Match> for [u8] {
    #[inline]
    fn index_mut(&mut self, index: Match) -> &mut [u8] {
        &mut self[index.start..index.end]
    }
}

impl ops::Index<Match> for str {
    type Output = str;

    #[inline]
    fn index(&self, index: Match) -> &str {
        &self[index.start..index.end]
    }
}

/// A line terminator.
///
/// A line terminator represents the end of a line. Generally, every line is
/// either "terminated" by the end of a stream or a specific byte (or sequence
/// of bytes).
///
/// Generally, a line terminator is a single byte, specifically, `\n`, on
/// Unix-like systems. On Windows, a line terminator is `\r\n` (referred to
/// as `CRLF` for `Carriage Return; Line Feed`).
///
/// The default line terminator is `\n` on all platforms.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct LineTerminator(LineTerminatorImp);

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
enum LineTerminatorImp {
    /// Any single byte representing a line terminator.
    ///
    /// We represent this as an array so we can safely convert it to a slice
    /// for convenient access. At some point, we can use `std::slice::from_ref`
    /// instead.
    Byte([u8; 1]),
    /// A line terminator represented by `\r\n`.
    ///
    /// When this option is used, consumers may generally treat a lone `\n` as
    /// a line terminator in addition to `\r\n`.
    CRLF,
}

impl LineTerminator {
    /// Return a new single-byte line terminator. Any byte is valid.
    #[inline]
    pub fn byte(byte: u8) -> LineTerminator {
        LineTerminator(LineTerminatorImp::Byte([byte]))
    }

    /// Return a new line terminator represented by `\r\n`.
    ///
    /// When this option is used, consumers may generally treat a lone `\n` as
    /// a line terminator in addition to `\r\n`.
    #[inline]
    pub fn crlf() -> LineTerminator {
        LineTerminator(LineTerminatorImp::CRLF)
    }

    /// Returns true if and only if this line terminator is CRLF.
    #[inline]
    pub fn is_crlf(&self) -> bool {
        self.0 == LineTerminatorImp::CRLF
    }

    /// Returns this line terminator as a single byte.
    ///
    /// If the line terminator is CRLF, then this returns `\n`. This is
    /// useful for routines that, for example, find line boundaries by treating
    /// `\n` as a line terminator even when it isn't preceded by `\r`.
    #[inline]
    pub fn as_byte(&self) -> u8 {
        match self.0 {
            LineTerminatorImp::Byte(array) => array[0],
            LineTerminatorImp::CRLF => b'\n',
        }
    }

    /// Returns this line terminator as a sequence of bytes.
    ///
    /// This returns a singleton sequence for all line terminators except for
    /// `CRLF`, in which case, it returns `\r\n`.
    ///
    /// The slice returned is guaranteed to have length at least `1`.
    #[inline]
    pub fn as_bytes(&self) -> &[u8] {
        match self.0 {
            LineTerminatorImp::Byte(ref array) => array,
            LineTerminatorImp::CRLF => &[b'\r', b'\n'],
        }
    }

    /// Returns true if and only if the given slice ends with this line
    /// terminator.
    ///
    /// If this line terminator is `CRLF`, then this only checks whether the
    /// last byte is `\n`.
    #[inline]
    pub fn is_suffix(&self, slice: &[u8]) -> bool {
        slice.last().map_or(false, |&b| b == self.as_byte())
    }
}

impl Default for LineTerminator {
    #[inline]
    fn default() -> LineTerminator {
        LineTerminator::byte(b'\n')
    }
}
