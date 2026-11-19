use std::io;

use cloneyard_api::{Method, MethodError, NoError};
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

/// Why a call failed; `E` holds the errors of the method itself.
#[derive(Debug, thiserror::Error)]
pub enum ClientError<E = NoError> {
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
    #[error(transparent)]
    Method(E),
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

    /// Calls `M` with its params and gives its result.
    ///
    /// # Errors
    ///
    /// Returns [`ClientError::Method`] with one of the method's own errors, or fails as [`Client::call`] otherwise.
    pub async fn request<M: Method>(
        &mut self,
        params: M::Params,
    ) -> Result<M::Result, ClientError<M::Error>> {
        match self.round_trip(M::NAME, params).await? {
            Ok(value) => serde_json::from_value(value).map_err(ClientError::InvalidResponse),
            Err(error) => Err(match M::Error::decode(error.code, error.data.clone()) {
                Some(Ok(error)) => ClientError::Method(error),
                Some(Err(invalid)) => ClientError::InvalidResponse(invalid),
                None => ClientError::Rpc(error),
            }),
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
        match self.round_trip(method, params).await? {
            Ok(value) => serde_json::from_value(value).map_err(ClientError::InvalidResponse),
            Err(error) => Err(ClientError::Rpc(error)),
        }
    }

    /// Sends one request and gives the result or the error object that answers it.
    async fn round_trip<P: Serialize, E>(
        &mut self,
        method: &str,
        params: P,
    ) -> Result<Result<Value, Error>, ClientError<E>> {
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
            return Ok(match response.outcome {
                Outcome::Result(value) => Ok(value),
                Outcome::Error(error) => Err(error),
            });
        }
    }
}
