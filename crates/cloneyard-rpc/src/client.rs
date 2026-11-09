use std::io;

use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;
use tokio::io::{AsyncRead, AsyncWrite, BufReader};

use crate::Error;
use crate::frame::{self, Frame};
use crate::message::{Id, Outcome, Request, Response, V2};

/// Calls methods one at a time over a reader and writer pair.
pub struct Client<R, W> {
    reader: BufReader<R>,
    writer: W,
    next_id: i64,
}

#[derive(Debug, thiserror::Error)]
pub enum ClientError {
    #[error(transparent)]
    Io(#[from] io::Error),
    #[error("the connection closed before the response")]
    Closed,
    #[error("cannot encode the params: {0}")]
    Params(serde_json::Error),
    #[error("invalid response: {0}")]
    InvalidResponse(serde_json::Error),
    #[error(transparent)]
    Rpc(#[from] Error),
}

impl<R, W> Client<R, W>
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    pub fn new(reader: R, writer: W) -> Self {
        Self {
            reader: BufReader::new(reader),
            writer,
            next_id: 1,
        }
    }

    /// Calls `method`; params that serialize to `null`, like `()`, are left out.
    ///
    /// # Errors
    ///
    /// Returns [`ClientError::Rpc`] with the error object the other side answered with,
    /// or another variant when the call didn't get a valid answer.
    pub async fn call<P, T>(&mut self, method: &str, params: P) -> Result<T, ClientError>
    where
        P: Serialize,
        T: DeserializeOwned,
    {
        let id = Id::Number(self.next_id.into());
        self.next_id += 1;
        let params = match serde_json::to_value(params).map_err(ClientError::Params)? {
            Value::Null => None,
            params => Some(params),
        };
        let request = Request {
            jsonrpc: V2,
            method: method.to_owned(),
            params,
            id: Some(id.clone()),
        };
        frame::write(&mut self.writer, &request).await?;
        loop {
            let line = match frame::read(&mut self.reader).await? {
                Frame::Line(line) => line,
                Frame::Eof => return Err(ClientError::Closed),
                Frame::TooLong => {
                    return Err(
                        io::Error::new(io::ErrorKind::InvalidData, frame::too_long()).into(),
                    );
                }
            };
            let response: Response =
                serde_json::from_slice(&line).map_err(ClientError::InvalidResponse)?;
            // With one call in flight, an error the server couldn't tie to an id is ours.
            if response.id != id && response.id != Id::Null {
                continue;
            }
            return match response.outcome {
                Outcome::Result(value) => {
                    serde_json::from_value(value).map_err(ClientError::InvalidResponse)
                }
                Outcome::Error(error) => Err(ClientError::Rpc(error)),
            };
        }
    }
}
