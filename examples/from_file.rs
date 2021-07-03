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

    // The builder is overly verbose just to demonstrate options
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
