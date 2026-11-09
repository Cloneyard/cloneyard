use std::io;

use tokio::io::{AsyncRead, AsyncWrite, BufReader};

use crate::frame::{self, Frame};
use crate::message::{self, Id, Request, Response};
use crate::{Error, Router};

/// Answers requests from `reader` on `writer`, one at a time, until `reader` ends.
///
/// # Errors
///
/// Returns the I/O error that broke the connection.
pub async fn serve<R, W>(router: &Router, reader: R, mut writer: W) -> io::Result<()>
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    let mut reader = BufReader::new(reader);
    loop {
        let response = match frame::read(&mut reader).await? {
            Frame::Eof => return Ok(()),
            Frame::TooLong => Some(Response::new(
                Id::Null,
                Err(Error::invalid_request(frame::too_long())),
            )),
            Frame::Line(line) if line.iter().all(u8::is_ascii_whitespace) => None,
            Frame::Line(line) => answer(router, &line).await,
        };
        if let Some(response) = response {
            frame::write(&mut writer, &response).await?;
        }
    }
}

async fn answer(router: &Router, line: &[u8]) -> Option<Response> {
    match message::parse_request(line) {
        Err((id, error)) => Some(Response::new(id, Err(error))),
        Ok(Request {
            method,
            params,
            id: None,
            ..
        }) => {
            if let Err(error) = router.call(&method, params).await {
                tracing::debug!(method, %error, "notification failed");
            }
            None
        }
        Ok(Request {
            method,
            params,
            id: Some(id),
            ..
        }) => Some(Response::new(id, router.call(&method, params).await)),
    }
}
