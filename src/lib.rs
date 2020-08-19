
use std::sync::{PoisonError, mpsc::{SendError, RecvError}};
use std::{any::Any, io};
use serde::{Serialize, Deserialize, de::DeserializeOwned};
use serde_json;

mod client;
mod server;

pub use server::{Server, ClientResponse};
pub use client::{Client, ServerResponse};

// -----------------------------------------------------------------------------------------------
// TYPE ALIASES
// -----------------------------------------------------------------------------------------------

pub type NetResult<T> = std::result::Result<T, NetError>;

// -----------------------------------------------------------------------------------------------
// STRUCTS
// -----------------------------------------------------------------------------------------------

#[derive(Debug, Serialize, Deserialize)]
pub struct Frame<T> {
    pub frame_counter: u64,
    pub data: T
}

// -----------------------------------------------------------------------------------------------
// ENUMS
// -----------------------------------------------------------------------------------------------

#[derive(Debug, thiserror::Error)]
pub enum NetError {
    #[error("An internal channel was disconnected")]
    ChannelDisconnected,

    #[error("There was an IO error: {0}")]
    IoError(io::Error),

    #[error("The server was unable to bind to the given port")]
    BindError,

    #[error("The connection attempt was refused by the peer")]
    ConnectionRefused,

    #[error("A spawned thread panic and returned an error when joined: {0:?}")]
    JoinError(Box<dyn Any + Send + 'static>),

    #[error("An internal mutex was poisoned")]
    MutexPoisoned,

    #[error("A channel endpoint delivered an unexpected response")]
    UnexpectedResponse,

    #[error("The background thread is not running")]
    NotRunning,

    #[error("Error serialising to JSON: {0}")]
    SerializationError(serde_json::Error),

    #[error("Error deserialising from JSON: {0}")]
    DesrializationError(serde_json::Error)
}

// -----------------------------------------------------------------------------------------------
// IMPLS
// -----------------------------------------------------------------------------------------------

impl From<io::Error> for NetError {
    fn from(error: io::Error) -> Self {
        NetError::IoError(error)
    }
}

impl<T> From<SendError<T>> for NetError {
    fn from(_: SendError<T>) -> Self {
        NetError::ChannelDisconnected
    }
}

impl From<RecvError> for NetError {
    fn from(_: RecvError) -> Self {
        NetError::ChannelDisconnected
    }
}

impl<T> From<PoisonError<T>> for NetError {
    fn from(_: PoisonError<T>) -> Self {
        NetError::MutexPoisoned
    }
}

impl<T: DeserializeOwned> Frame<T> {
    pub fn from_bytes(bytes: &[u8]) -> NetResult<Self> {
        match serde_json::from_slice(bytes) {
            Ok(d) => Ok(d),
            Err(e) => Err(NetError::DesrializationError(e))
        }
    }
}

impl<T: Serialize> Frame<T> {
    pub fn to_byte_vec(&self) -> NetResult<Vec<u8>> {
        match serde_json::to_vec(self) {
            Ok(v) => Ok(v),
            Err(e) => Err(NetError::SerializationError(e))
        }
    }
}
