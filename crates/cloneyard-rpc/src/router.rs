use std::collections::HashMap;
use std::future::{self, Future};
use std::pin::Pin;

use cloneyard_api::Method;
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::{Map, Value};

use crate::Error;

type Reply = Pin<Box<dyn Future<Output = Result<Value, Error>> + Send>>;
type BoxedHandler = Box<dyn Fn(Value) -> Reply + Send + Sync>;

/// An async function from params `P` to a result `R`, such as `|(): ()| async { Ok(1) }`.
// `Fn` is the supertrait so that a closure passed as `impl Handler` gets its argument type from `P`.
pub trait Handler<P, R>: Fn(P) -> Self::Reply + Send + Sync + 'static {
    type Reply: Future<Output = Result<R, Error>> + Send + 'static;
}

impl<P, R, F, Fut> Handler<P, R> for F
where
    F: Fn(P) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = Result<R, Error>> + Send + 'static,
{
    type Reply = Fut;
}

#[derive(Default)]
pub struct Router {
    handlers: HashMap<String, BoxedHandler>,
}

impl Router {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds `M`, whose handler takes its params and returns its result.
    #[must_use]
    pub fn handle<M: Method>(self, handler: impl Handler<M::Params, M::Result>) -> Self {
        self.method(M::NAME, handler)
    }

    /// The names of the methods this router answers, sorted.
    #[must_use]
    pub fn methods(&self) -> Vec<&str> {
        let mut names: Vec<&str> = self.handlers.keys().map(String::as_str).collect();
        names.sort_unstable();
        names
    }

    /// Adds a method whose params deserialize into `P`; a method without params takes `()`.
    #[must_use]
    pub fn method<P, R, F, Fut>(mut self, name: &str, handler: F) -> Self
    where
        P: DeserializeOwned,
        R: Serialize,
        F: Fn(P) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<R, Error>> + Send + 'static,
    {
        let handler = move |params: Value| -> Reply {
            match parse_params(params) {
                Ok(params) => {
                    let reply = handler(params);
                    Box::pin(
                        async move { serde_json::to_value(reply.await?).map_err(Error::internal) },
                    )
                }
                Err(error) => Box::pin(future::ready(Err(Error::invalid_params(error)))),
            }
        };
        self.handlers.insert(name.to_owned(), Box::new(handler));
        self
    }

    pub(crate) async fn call(&self, method: &str, params: Option<Value>) -> Result<Value, Error> {
        tracing::debug!(method, "call");
        let handler = self
            .handlers
            .get(method)
            .ok_or_else(|| Error::method_not_found(method))?;
        handler(params.unwrap_or(Value::Null)).await
    }
}

// Clients often send `[]` or `{}` to a method without params, which `()` accepts only as null.
fn parse_params<P: DeserializeOwned>(params: Value) -> Result<P, serde_json::Error> {
    let empty = params.as_array().is_some_and(Vec::is_empty)
        || params.as_object().is_some_and(Map::is_empty);
    match serde_json::from_value(params) {
        Err(error) if empty => serde_json::from_value(Value::Null).map_err(|_| error),
        parsed => parsed,
    }
}
