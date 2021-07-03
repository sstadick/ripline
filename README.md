# 🌊 ripline

<p align="center">
  <a href="https://github.com/sstadick/ripline/actions?query=workflow%3Aci"><img src="https://github.com/sstadick/ripline/workflows/ci/badge.svg" alt="Build Status"></a>
  <img src="https://img.shields.io/crates/l/ripline.svg" alt="license">
  <a href="https://crates.io/crates/ripline"><img src="https://img.shields.io/crates/v/ripline.svg?colorB=319e8c" alt="Version info"></a><br>
 <i>This is not the greatest line reader in the world, this is just a tribute.</i>
</p>

Fast line based iteration almost entirely lifted from ripgrep's [grep_searcher](https://github.com/BurntSushi/ripgrep/tree/master/crates/searcher).

All credit to Andrew Gallant and the ripgrep contributors.

## Why?

- Doesn't rely on a clousre like the `bstr::for_line*` methods (useful in some award lifetime scenarios).
- No silently capped line lengths unlike `rust-linereader`
- Brings the `LineIter` with for working with `memmap` files

Not all of this functionality was exposed in the `grep_searcher` crate, and rightly so as a lot of it had `grep` specific configurations embeded into the logic (i.e. binary detection).

## What have I changed?

Not much. I took out some of the ripgrep specific logic such as the binary detection, some search related configs, and consolidated a few of the helper stucts from the other `grep_*` crates.

## Example

See `examples` for more.

```rust
use grep_cli::stdout;
use ripline::{
    line_buffer::{LineBufferBuilder, LineBufferReader},
    lines::LineIter,
    LineTerminator,
};
use std::{env, error::Error, fs::File, io::Write, path::PathBuf};
use termcolor::ColorChoice;

fn main() -> Result<(), Box<dyn Error>> {
    let path = PathBuf::from(env::args().nth(1).expect("Failed to provide input file"));

    let mut out = stdout(ColorChoice::Never);

    let reader = File::open(&path)?;
    let terminator = LineTerminator::byte(b'\n');
    let mut line_buffer = LineBufferBuilder::new().build();
    let mut lb_reader = LineBufferReader::new(reader, &mut line_buffer);

    while lb_reader.fill()? {
        let lines = LineIter::new(terminator.as_byte(), lb_reader.buffer());
        for line in lines {
            out.write_all(line)?;
        }
        lb_reader.consume_all();
    }

    Ok(())
}
```

## Crude and untrustworthy benchmarks

From `examples/ripline_benchmarks.rs`. Initial benchmark script take from [rust-linereader](https://github.com/Freaky/rust-linereader), which is also included in the benchmarks as `LR:*`.

The input used was [all_train.csv](https://archive.ics.uci.edu/ml/machine-learning-databases/00347/all_train.csv.gz), unzipped can catted together five times createing a ~25G file.

| Method                |  Time |  Lines/sec |     Bandwidth |
| --------------------- | ----: | ---------: | ------------: |
| read()                | 2.01s | 17439155/s | 12303.42 MB/s |
| LR::next_batch()      | 2.11s | 16576174/s | 11694.59 MB/s |
| LR::next_line()       | 2.65s | 13196734/s |  9310.37 MB/s |
| ripline_line_buffer() | 2.64s | 13277194/s |  9367.14 MB/s |
| ripline_mmap()        | 2.16s | 16183503/s | 11417.55 MB/s |
| bstr_for_line()       | 2.47s | 14174502/s | 10000.19 MB/s |
| read_until()          | 2.86s | 12230594/s |  8628.75 MB/s |
| read_line()           | 4.16s |  8417415/s |  5938.53 MB/s |
| lines()               | 5.05s |  6930901/s |  4889.79 MB/s |

Note that `read` and `next_batch` are not counting lines.

Hardware: Ubuntu 20 AMD Ryzen 9 3950X 16-Core Processor w/ 64 GB DDR4 memory and 1TB NVMe Drive
