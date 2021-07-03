use std::cmp;
use std::io;

use bstr::ByteSlice;

/// The default buffer capacity that we use for the line buffer.
pub(crate) const DEFAULT_BUFFER_CAPACITY: usize = 64 * (1 << 10); // 64 KB

/// The behavior of a searcher in the face of long lines and big contexts.
///
/// When searching data incrementally using a fixed size buffer, this controls
/// the amount of *additional* memory to allocate beyond the size of the buffer
/// to accommodate lines (which may include the lines in a context window, when
/// enabled) that do not fit in the buffer.
///
/// The default is to eagerly allocate without a limit.
#[derive(Clone, Copy, Debug)]
pub enum BufferAllocation {
    /// Attempt to expand the size of the buffer until either at least the next
    /// line fits into memory or until all available memory is exhausted.
    ///
    /// This is the default.
    Eager,
    /// Limit the amount of additional memory allocated to the given size. If
    /// a line is found that requires more memory than is allowed here, then
    /// stop reading and return an error.
    Error(usize),
}

impl Default for BufferAllocation {
    fn default() -> BufferAllocation {
        BufferAllocation::Eager
    }
}

/// Create a new error to be used when a configured allocation limit has been
/// reached.
pub fn alloc_error(limit: usize) -> io::Error {
    let msg = format!("configured allocation limit ({}) exceeded", limit);
    io::Error::new(io::ErrorKind::Other, msg)
}

/// The configuration of a buffer. This contains options that are fixed once
/// a buffer has been constructed.
#[derive(Clone, Copy, Debug)]
struct Config {
    /// The number of bytes to attempt to read at a time.
    capacity: usize,
    /// The line terminator.
    lineterm: u8,
    /// The behavior for handling long lines.
    buffer_alloc: BufferAllocation,
}

impl Default for Config {
    fn default() -> Config {
        Config {
            capacity: DEFAULT_BUFFER_CAPACITY,
            lineterm: b'\n',
            buffer_alloc: BufferAllocation::default(),
        }
    }
}

/// A builder for constructing line buffers.
#[derive(Clone, Debug, Default)]
pub struct LineBufferBuilder {
    config: Config,
}

impl LineBufferBuilder {
    /// Create a new builder for a buffer.
    pub fn new() -> LineBufferBuilder {
        LineBufferBuilder {
            config: Config::default(),
        }
    }

    /// Create a new line buffer from this builder's configuration.
    pub fn build(&self) -> LineBuffer {
        LineBuffer {
            config: self.config,
            buf: vec![0; self.config.capacity],
            pos: 0,
            last_lineterm: 0,
            end: 0,
            absolute_byte_offset: 0,
            binary_byte_offset: None,
        }
    }

    /// Set the default capacity to use for a buffer.
    ///
    /// In general, the capacity of a buffer corresponds to the amount of data
    /// to hold in memory, and the size of the reads to make to the underlying
    /// reader.
    ///
    /// This is set to a reasonable default and probably shouldn't be changed
    /// unless there's a specific reason to do so.
    pub fn capacity(&mut self, capacity: usize) -> &mut LineBufferBuilder {
        self.config.capacity = capacity;
        self
    }

    /// Set the line terminator for the buffer.
    ///
    /// Every buffer has a line terminator, and this line terminator is used
    /// to determine how to roll the buffer forward. For example, when a read
    /// to the buffer's underlying reader occurs, the end of the data that is
    /// read is likely to correspond to an incomplete line. As a line buffer,
    /// callers should not access this data since it is incomplete. The line
    /// terminator is how the line buffer determines the part of the read that
    /// is incomplete.
    ///
    /// By default, this is set to `b'\n'`.
    pub fn line_terminator(&mut self, lineterm: u8) -> &mut LineBufferBuilder {
        self.config.lineterm = lineterm;
        self
    }

    /// Set the maximum amount of additional memory to allocate for long lines.
    ///
    /// In order to enable line oriented search, a fundamental requirement is
    /// that, at a minimum, each line must be able to fit into memory. This
    /// setting controls how big that line is allowed to be. By default, this
    /// is set to `BufferAllocation::Eager`, which means a line buffer will
    /// attempt to allocate as much memory as possible to fit a line, and will
    /// only be limited by available memory.
    ///
    /// Note that this setting only applies to the amount of *additional*
    /// memory to allocate, beyond the capacity of the buffer. That means that
    /// a value of `0` is sensible, and in particular, will guarantee that a
    /// line buffer will never allocate additional memory beyond its initial
    /// capacity.
    pub fn buffer_alloc(&mut self, behavior: BufferAllocation) -> &mut LineBufferBuilder {
        self.config.buffer_alloc = behavior;
        self
    }
}

/// A line buffer reader efficiently reads a line oriented buffer from an
/// arbitrary reader.
#[derive(Debug)]
pub struct LineBufferReader<'b, R> {
    rdr: R,
    line_buffer: &'b mut LineBuffer,
}

impl<'b, R: io::Read> LineBufferReader<'b, R> {
    /// Create a new buffered reader that reads from `rdr` and uses the given
    /// `line_buffer` as an intermediate buffer.
    ///
    /// This does not change the binary detection behavior of the given line
    /// buffer.
    pub fn new(rdr: R, line_buffer: &'b mut LineBuffer) -> LineBufferReader<'b, R> {
        line_buffer.clear();
        LineBufferReader { rdr, line_buffer }
    }

    /// The absolute byte offset which corresponds to the starting offsets
    /// of the data returned by `buffer` relative to the beginning of the
    /// underlying reader's contents. As such, this offset does not generally
    /// correspond to an offset in memory. It is typically used for reporting
    /// purposes. It can also be used for counting the number of bytes that
    /// have been searched.
    pub fn absolute_byte_offset(&self) -> u64 {
        self.line_buffer.absolute_byte_offset()
    }

    /// If binary data was detected, then this returns the absolute byte offset
    /// at which binary data was initially found.
    pub fn binary_byte_offset(&self) -> Option<u64> {
        self.line_buffer.binary_byte_offset()
    }

    /// Fill the contents of this buffer by discarding the part of the buffer
    /// that has been consumed. The free space created by discarding the
    /// consumed part of the buffer is then filled with new data from the
    /// reader.
    ///
    /// If EOF is reached, then `false` is returned. Otherwise, `true` is
    /// returned. (Note that if this line buffer's binary detection is set to
    /// `Quit`, then the presence of binary data will cause this buffer to
    /// behave as if it had seen EOF at the first occurrence of binary data.)
    ///
    /// This forwards any errors returned by the underlying reader, and will
    /// also return an error if the buffer must be expanded past its allocation
    /// limit, as governed by the buffer allocation strategy.
    pub fn fill(&mut self) -> Result<bool, io::Error> {
        self.line_buffer.fill(&mut self.rdr)
    }

    /// Return the contents of this buffer.
    pub fn buffer(&self) -> &[u8] {
        self.line_buffer.buffer()
    }

    /// Return the buffer as a BStr, used for convenient equality checking
    pub fn bstr(&self) -> &::bstr::BStr {
        self.buffer().as_bstr()
    }

    /// Consume the number of bytes provided. This must be less than or equal
    /// to the number of bytes returned by `buffer`.
    pub fn consume(&mut self, amt: usize) {
        self.line_buffer.consume(amt);
    }

    /// Consumes the remainder of the buffer. Subsequent calls to `buffer` are
    /// guaranteed to return an empty slice until the buffer is refilled.
    ///
    /// This is a convenience function for `consume(buffer.len())`.
    pub fn consume_all(&mut self) {
        self.line_buffer.consume_all();
    }
}

/// A line buffer manages a (typically fixed) buffer for holding lines.
///
/// Callers should create line buffers sparingly and reuse them when possible.
/// Line buffers cannot be used directly, but instead must be used via the
/// LineBufferReader.
#[derive(Clone, Debug)]
pub struct LineBuffer {
    /// The configuration of this buffer.
    config: Config,
    /// The primary buffer with which to hold data.
    buf: Vec<u8>,
    /// The current position of this buffer. This is always a valid sliceable
    /// index into `buf`, and its maximum value is the length of `buf`.
    pos: usize,
    /// The end position of searchable content in this buffer. This is either
    /// set to just after the final line terminator in the buffer, or to just
    /// after the end of the last byte emitted by the reader when the reader
    /// has been exhausted.
    last_lineterm: usize,
    /// The end position of the buffer. This is always greater than or equal to
    /// last_lineterm. The bytes between last_lineterm and end, if any, always
    /// correspond to a partial line.
    end: usize,
    /// The absolute byte offset corresponding to `pos`. This is most typically
    /// not a valid index into addressable memory, but rather, an offset that
    /// is relative to all data that passes through a line buffer (since
    /// construction or since the last time `clear` was called).
    ///
    /// When the line buffer reaches EOF, this is set to the position just
    /// after the last byte read from the underlying reader. That is, it
    /// becomes the total count of bytes that have been read.
    absolute_byte_offset: u64,
    /// If binary data was found, this records the absolute byte offset at
    /// which it was first detected.
    binary_byte_offset: Option<u64>,
}

impl LineBuffer {
    /// Reset this buffer, such that it can be used with a new reader.
    fn clear(&mut self) {
        self.pos = 0;
        self.last_lineterm = 0;
        self.end = 0;
        self.absolute_byte_offset = 0;
        self.binary_byte_offset = None;
    }

    /// The absolute byte offset which corresponds to the starting offsets
    /// of the data returned by `buffer` relative to the beginning of the
    /// reader's contents. As such, this offset does not generally correspond
    /// to an offset in memory. It is typically used for reporting purposes,
    /// particularly in error messages.
    ///
    /// This is reset to `0` when `clear` is called.
    fn absolute_byte_offset(&self) -> u64 {
        self.absolute_byte_offset
    }

    /// If binary data was detected, then this returns the absolute byte offset
    /// at which binary data was initially found.
    fn binary_byte_offset(&self) -> Option<u64> {
        self.binary_byte_offset
    }

    /// Return the contents of this buffer.
    fn buffer(&self) -> &[u8] {
        &self.buf[self.pos..self.last_lineterm]
    }

    /// Return the contents of the free space beyond the end of the buffer as
    /// a mutable slice.
    fn free_buffer(&mut self) -> &mut [u8] {
        &mut self.buf[self.end..]
    }

    /// Consume the number of bytes provided. This must be less than or equal
    /// to the number of bytes returned by `buffer`.
    fn consume(&mut self, amt: usize) {
        assert!(amt <= self.buffer().len());
        self.pos += amt;
        self.absolute_byte_offset += amt as u64;
    }

    /// Consumes the remainder of the buffer. Subsequent calls to `buffer` are
    /// guaranteed to return an empty slice until the buffer is refilled.
    ///
    /// This is a convenience function for `consume(buffer.len())`.
    fn consume_all(&mut self) {
        let amt = self.buffer().len();
        self.consume(amt);
    }

    /// Fill the contents of this buffer by discarding the part of the buffer
    /// that has been consumed. The free space created by discarding the
    /// consumed part of the buffer is then filled with new data from the given
    /// reader.
    ///
    /// Callers should provide the same reader to this line buffer in
    /// subsequent calls to fill. A different reader can only be used
    /// immediately following a call to `clear`.
    ///
    /// If EOF is reached, then `false` is returned. Otherwise, `true` is
    /// returned.
    ///
    /// This forwards any errors returned by `rdr`, and will also return an
    /// error if the buffer must be expanded past its allocation limit, as
    /// governed by the buffer allocation strategy.
    fn fill<R: io::Read>(&mut self, mut rdr: R) -> Result<bool, io::Error> {
        self.roll();
        assert_eq!(self.pos, 0);
        loop {
            self.ensure_capacity()?;
            let readlen = rdr.read(self.free_buffer().as_bytes_mut())?;
            if readlen == 0 {
                // We're only done reading for good once the caller has
                // consumed everything.
                self.last_lineterm = self.end;
                return Ok(!self.buffer().is_empty());
            }

            // Get a mutable view into the bytes we've just read. These are
            // the bytes that we do binary detection on, and also the bytes we
            // search to find the last line terminator. We need a mutable slice
            // in the case of binary conversion.
            let oldend = self.end;
            self.end += readlen;
            let newbytes = &mut self.buf[oldend..self.end];

            // Update our `last_lineterm` positions if we read one.
            if let Some(i) = newbytes.rfind_byte(self.config.lineterm) {
                self.last_lineterm = oldend + i + 1;
                return Ok(true);
            }
            // At this point, if we couldn't find a line terminator, then we
            // don't have a complete line. Therefore, we try to read more!
        }
    }

    /// Roll the unconsumed parts of the buffer to the front.
    ///
    /// This operation is idempotent.
    ///
    /// After rolling, `last_lineterm` and `end` point to the same location,
    /// and `pos` is always set to `0`.
    fn roll(&mut self) {
        if self.pos == self.end {
            self.pos = 0;
            self.last_lineterm = 0;
            self.end = 0;
            return;
        }

        let roll_len = self.end - self.pos;
        self.buf.copy_within_str(self.pos..self.end, 0);
        self.pos = 0;
        self.last_lineterm = roll_len;
        self.end = roll_len;
    }

    /// Ensures that the internal buffer has a non-zero amount of free space
    /// in which to read more data. If there is no free space, then more is
    /// allocated. If the allocation must exceed the configured limit, then
    /// this returns an error.
    fn ensure_capacity(&mut self) -> Result<(), io::Error> {
        if !self.free_buffer().is_empty() {
            return Ok(());
        }
        // `len` is used for computing the next allocation size. The capacity
        // is permitted to start at `0`, so we make sure it's at least `1`.
        let len = cmp::max(1, self.buf.len());
        let additional = match self.config.buffer_alloc {
            BufferAllocation::Eager => len * 2,
            BufferAllocation::Error(limit) => {
                let used = self.buf.len() - self.config.capacity;
                let n = cmp::min(len * 2, limit - used);
                if n == 0 {
                    return Err(alloc_error(self.config.capacity + limit));
                }
                n
            }
        };
        assert!(additional > 0);
        let newlen = self.buf.len() + additional;
        self.buf.resize(newlen, 0);
        assert!(!self.free_buffer().is_empty());
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bstr::{ByteSlice, ByteVec};

    #[test]
    fn buffer_basics1() {
        let bytes = "homer\nlisa\nmaggie";
        let mut linebuf = LineBufferBuilder::new().build();
        let mut rdr = LineBufferReader::new(bytes.as_bytes(), &mut linebuf);

        assert!(rdr.buffer().is_empty());

        assert!(rdr.fill().unwrap());
        assert_eq!(rdr.bstr(), "homer\nlisa\n");
        assert_eq!(rdr.absolute_byte_offset(), 0);
        rdr.consume(5);
        assert_eq!(rdr.absolute_byte_offset(), 5);
        rdr.consume_all();
        assert_eq!(rdr.absolute_byte_offset(), 11);

        assert!(rdr.fill().unwrap());
        assert_eq!(rdr.bstr(), "maggie");
        rdr.consume_all();

        assert!(!rdr.fill().unwrap());
        assert_eq!(rdr.absolute_byte_offset(), bytes.len() as u64);
        assert_eq!(rdr.binary_byte_offset(), None);
    }

    #[test]
    fn buffer_basics2() {
        let bytes = "homer\nlisa\nmaggie\n";
        let mut linebuf = LineBufferBuilder::new().build();
        let mut rdr = LineBufferReader::new(bytes.as_bytes(), &mut linebuf);

        assert!(rdr.fill().unwrap());
        assert_eq!(rdr.bstr(), "homer\nlisa\nmaggie\n");
        rdr.consume_all();

        assert!(!rdr.fill().unwrap());
        assert_eq!(rdr.absolute_byte_offset(), bytes.len() as u64);
        assert_eq!(rdr.binary_byte_offset(), None);
    }

    #[test]
    fn buffer_basics3() {
        let bytes = "\n";
        let mut linebuf = LineBufferBuilder::new().build();
        let mut rdr = LineBufferReader::new(bytes.as_bytes(), &mut linebuf);

        assert!(rdr.fill().unwrap());
        assert_eq!(rdr.bstr(), "\n");
        rdr.consume_all();

        assert!(!rdr.fill().unwrap());
        assert_eq!(rdr.absolute_byte_offset(), bytes.len() as u64);
        assert_eq!(rdr.binary_byte_offset(), None);
    }

    #[test]
    fn buffer_basics4() {
        let bytes = "\n\n";
        let mut linebuf = LineBufferBuilder::new().build();
        let mut rdr = LineBufferReader::new(bytes.as_bytes(), &mut linebuf);

        assert!(rdr.fill().unwrap());
        assert_eq!(rdr.bstr(), "\n\n");
        rdr.consume_all();

        assert!(!rdr.fill().unwrap());
        assert_eq!(rdr.absolute_byte_offset(), bytes.len() as u64);
        assert_eq!(rdr.binary_byte_offset(), None);
    }

    #[test]
    fn buffer_empty() {
        let bytes = "";
        let mut linebuf = LineBufferBuilder::new().build();
        let mut rdr = LineBufferReader::new(bytes.as_bytes(), &mut linebuf);

        assert!(!rdr.fill().unwrap());
        assert_eq!(rdr.absolute_byte_offset(), bytes.len() as u64);
        assert_eq!(rdr.binary_byte_offset(), None);
    }

    #[test]
    fn buffer_zero_capacity() {
        let bytes = "homer\nlisa\nmaggie";
        let mut linebuf = LineBufferBuilder::new().capacity(0).build();
        let mut rdr = LineBufferReader::new(bytes.as_bytes(), &mut linebuf);

        while rdr.fill().unwrap() {
            rdr.consume_all();
        }
        assert_eq!(rdr.absolute_byte_offset(), bytes.len() as u64);
        assert_eq!(rdr.binary_byte_offset(), None);
    }

    #[test]
    fn buffer_small_capacity() {
        let bytes = "homer\nlisa\nmaggie";
        let mut linebuf = LineBufferBuilder::new().capacity(1).build();
        let mut rdr = LineBufferReader::new(bytes.as_bytes(), &mut linebuf);

        let mut got = vec![];
        while rdr.fill().unwrap() {
            got.push_str(rdr.buffer());
            rdr.consume_all();
        }
        assert_eq!(bytes, got.as_bstr());
        assert_eq!(rdr.absolute_byte_offset(), bytes.len() as u64);
        assert_eq!(rdr.binary_byte_offset(), None);
    }

    #[test]
    fn buffer_limited_capacity1() {
        let bytes = "homer\nlisa\nmaggie";
        let mut linebuf = LineBufferBuilder::new()
            .capacity(1)
            .buffer_alloc(BufferAllocation::Error(5))
            .build();
        let mut rdr = LineBufferReader::new(bytes.as_bytes(), &mut linebuf);

        assert!(rdr.fill().unwrap());
        assert_eq!(rdr.bstr(), "homer\n");
        rdr.consume_all();

        assert!(rdr.fill().unwrap());
        assert_eq!(rdr.bstr(), "lisa\n");
        rdr.consume_all();

        // This returns an error because while we have just enough room to
        // store maggie in the buffer, we *don't* have enough room to read one
        // more byte, so we don't know whether we're at EOF or not, and
        // therefore must give up.
        assert!(rdr.fill().is_err());

        // We can mush on though!
        assert_eq!(rdr.bstr(), "m");
        rdr.consume_all();

        assert!(rdr.fill().unwrap());
        assert_eq!(rdr.bstr(), "aggie");
        rdr.consume_all();

        assert!(!rdr.fill().unwrap());
    }

    #[test]
    fn buffer_limited_capacity2() {
        let bytes = "homer\nlisa\nmaggie";
        let mut linebuf = LineBufferBuilder::new()
            .capacity(1)
            .buffer_alloc(BufferAllocation::Error(6))
            .build();
        let mut rdr = LineBufferReader::new(bytes.as_bytes(), &mut linebuf);

        assert!(rdr.fill().unwrap());
        assert_eq!(rdr.bstr(), "homer\n");
        rdr.consume_all();

        assert!(rdr.fill().unwrap());
        assert_eq!(rdr.bstr(), "lisa\n");
        rdr.consume_all();

        // We have just enough space.
        assert!(rdr.fill().unwrap());
        assert_eq!(rdr.bstr(), "maggie");
        rdr.consume_all();

        assert!(!rdr.fill().unwrap());
    }

    #[test]
    fn buffer_limited_capacity3() {
        let bytes = "homer\nlisa\nmaggie";
        let mut linebuf = LineBufferBuilder::new()
            .capacity(1)
            .buffer_alloc(BufferAllocation::Error(0))
            .build();
        let mut rdr = LineBufferReader::new(bytes.as_bytes(), &mut linebuf);

        assert!(rdr.fill().is_err());
        assert_eq!(rdr.bstr(), "");
    }
}
