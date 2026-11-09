use std::io;

use serde::Serialize;
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncReadExt, AsyncWrite, AsyncWriteExt};

const MAX_LINE_MIB: usize = 16;
const MAX_LINE_BYTES: usize = MAX_LINE_MIB * 1024 * 1024;

pub(crate) fn too_long() -> String {
    format!("the line is longer than {MAX_LINE_MIB} MiB")
}

pub(crate) enum Frame {
    Line(Vec<u8>),
    TooLong,
    Eof,
}

pub(crate) async fn read<R: AsyncBufRead + Unpin>(reader: &mut R) -> io::Result<Frame> {
    let mut line = Vec::new();
    let limit = MAX_LINE_BYTES as u64 + 1;
    let read = (&mut *reader)
        .take(limit)
        .read_until(b'\n', &mut line)
        .await?;
    if read == 0 {
        return Ok(Frame::Eof);
    }
    if line.last() == Some(&b'\n') {
        line.pop();
        if line.last() == Some(&b'\r') {
            line.pop();
        }
    } else if line.len() > MAX_LINE_BYTES {
        skip_rest_of_line(reader).await?;
        return Ok(Frame::TooLong);
    }
    Ok(Frame::Line(line))
}

async fn skip_rest_of_line<R: AsyncBufRead + Unpin>(reader: &mut R) -> io::Result<()> {
    loop {
        let buffer = reader.fill_buf().await?;
        if buffer.is_empty() {
            return Ok(());
        }
        if let Some(newline) = buffer.iter().position(|&byte| byte == b'\n') {
            reader.consume(newline + 1);
            return Ok(());
        }
        let skipped = buffer.len();
        reader.consume(skipped);
    }
}

pub(crate) async fn write<W, T>(writer: &mut W, message: &T) -> io::Result<()>
where
    W: AsyncWrite + Unpin,
    T: Serialize,
{
    let mut line = serde_json::to_vec(message)?;
    line.push(b'\n');
    writer.write_all(&line).await?;
    writer.flush().await
}
