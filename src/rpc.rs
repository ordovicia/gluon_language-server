use std::error::Error as StdError;
use std::io::{BufRead, Read, Write};
use std::sync::Arc;
use std::sync::atomic::{self, AtomicBool};

use jsonrpc_core::{Error, ErrorCode, IoHandler, RpcMethodSimple, Params, Value};
use futures::{self, BoxFuture, Future, IntoFuture};

use serde;
use serde_json::{from_value, to_value};

pub struct ServerError<E> {
    pub message: String,
    pub data: Option<E>,
}

pub trait LanguageServerCommand: Send + Sync + 'static {
    type Param: serde::Deserialize;
    type Output: serde::Serialize;
    type Error: serde::Serialize;
    fn execute(&self, param: Self::Param) -> BoxFuture<Self::Output, ServerError<Self::Error>>;

    fn invalid_params(&self) -> Option<Self::Error> {
        None
    }
}

pub trait LanguageServerNotification: Send + Sync + 'static {
    type Param: serde::Deserialize;
    fn execute(&self, param: Self::Param);
}

pub struct ServerCommand<T>(pub T);

impl<T> RpcMethodSimple for ServerCommand<T>
    where T: LanguageServerCommand,
{
    fn call(&self, param: Params) -> BoxFuture<Value, Error> {
        match param {
            Params::Map(ref map) => {
                match from_value(Value::Object(map.clone())) {
                    Ok(value) => {
                        return self.0
                            .execute(value)
                            .then(|result| match result {
                                Ok(value) => {
                                    Ok(to_value(&value)
                                            .expect("result data could not be serialized"))
                                        .into_future()
                                }
                                Err(error) => {
                                    Err(Error {
                                            code: ErrorCode::InternalError,
                                            message: error.message,
                                            data: error.data
                                                .as_ref()
                                                .map(|v| {
                                                    to_value(v).expect("error data could not be \
                                                                        serialized")
                                                }),
                                        })
                                        .into_future()
                                }
                            })
                            .boxed()
                    }
                    Err(_) => (),
                }
            }
            _ => (),
        }
        let data = self.0.invalid_params();
        futures::failed(Error {
                code: ErrorCode::InvalidParams,
                message: format!("Invalid params: {:?}", param),
                data: data.as_ref()
                    .map(|v| to_value(v).expect("error data could not be serialized")),
            })
            .boxed()
    }
}

pub fn read_message<R>(mut reader: R) -> Result<Option<String>, Box<StdError>>
    where R: BufRead + Read,
{
    let mut header = String::new();
    let n = try!(reader.read_line(&mut header));
    if n == 0 {
        // EOF
        return Ok(None);
    }
    if header.starts_with("Content-Length: ") {
        let content_length = {
            let len = header["Content-Length:".len()..].trim();
            debug!("{}", len);
            try!(len.parse::<usize>())
        };
        while header != "\r\n" {
            header.clear();
            try!(reader.read_line(&mut header));
        }
        let mut content = vec![0; content_length];
        try!(reader.read_exact(&mut content));
        Ok(Some(try!(String::from_utf8(content))))
    } else {
        Err(format!("Invalid message: `{}`", header).into())
    }
}

pub fn main_loop<R, W, F, G>(mut input: R,
                             mut output: W,
                             io: &IoHandler,
                             exit_token: Arc<AtomicBool>,
                             mut map_request: F,
                             mut map_response: G)
                             -> Result<(), Box<StdError>>
    where R: BufRead,
          W: Write,
          F: FnMut(String) -> Result<String, Box<StdError>>,
          G: FnMut(String) -> Result<String, Box<StdError>>,
{
    while !exit_token.load(atomic::Ordering::SeqCst) {
        match try!(read_message(&mut input)) {
            Some(json) => {
                let json = try!(map_request(json));
                debug!("Handle: {}", json);
                if let Some(response) = io.handle_request_sync(&json) {
                    let response = try!(map_response(response));
                    try!(write!(output,
                                "Content-Length: {}\r\n\r\n{}",
                                response.len(),
                                response));
                    try!(output.flush());
                }
            }
            None => return Ok(()),
        }
    }
    Ok(())
}