
// -----------------------------------------------------------------------------------------------
// IMPORTS
// -----------------------------------------------------------------------------------------------

use std::{
    thread::{self, JoinHandle}, 
    sync::mpsc::{channel, Sender, Receiver, TryRecvError},
    net::{TcpStream, Shutdown}, 
    io::{self, Write},
};
use serde::{Serialize, de::DeserializeOwned};
use buffer::ReadBuffer;
use super::{Frame, NetResult, NetError};
use io::ErrorKind;

// -----------------------------------------------------------------------------------------------
// STRUCTS
// -----------------------------------------------------------------------------------------------

pub struct Client<S, R>
where
    S: Serialize + Send,
    R: DeserializeOwned + Send
{
    thread_handle: Option<JoinHandle<NetResult<()>>>,
    send_channel: Sender<ClientCommand<S>>,
    recv_channel: Receiver<ClientResponse<R>>
}

// -----------------------------------------------------------------------------------------------
// ENUMS
// -----------------------------------------------------------------------------------------------

#[derive(Debug)]
pub enum ServerResponse<R>
where
    R: DeserializeOwned + Send 
{
    None,
    Received(R),
    ReceivedOutOfOrder(R),
    Error(NetError),
    Disconnected
}

enum ClientCommand<S>
where
    S: Serialize + Send
{
    Shutdown,
    Send(S)
}

enum ClientResponse<R>
where
    R: DeserializeOwned + Send
{
    Received(R),
    ReceivedOutOfOrder(R),
    Error(NetError),
    Disconnected
}

// -----------------------------------------------------------------------------------------------
// IMPLS
// -----------------------------------------------------------------------------------------------

impl<S, R> Client<S, R>
where 
    S: Serialize + Send + 'static,
    R: DeserializeOwned + Send + 'static
{
    /// Connect to a server at the given port number.
    pub fn connect(port_num: u16) -> NetResult<Self> {
        // Attempt to connect to the server
        let stream = match TcpStream::connect(("0.0.0.0", port_num)) {
            Ok(s) => s,
            Err(e) => match e.kind() {
                ErrorKind::ConnectionRefused => return Err(NetError::ConnectionRefused),
                _ => return Err(e.into())
            }
        };

        // Create channel endpoints
        let (send_tx, send_rx) = channel::<ClientCommand<S>>();
        let (recv_tx, recv_rx) = channel::<ClientResponse<R>>();

        // Start the background thread
        let thread_handle = thread::spawn(move || {
            client(stream, send_rx, recv_tx)
        });

        // Return the endpoint
        Ok(Self {
            thread_handle: Some(thread_handle),
            send_channel: send_tx,
            recv_channel: recv_rx
        })
    }

    /// Shutdown the client.
    pub fn shutdown(mut self) -> NetResult<()> {
        // Send the shutdown command to the threads
        self.send_channel.send(ClientCommand::Shutdown)?;

        // Wait for the listener thread to stop
        if let Some(ljh) = self.thread_handle.take() {
            match ljh.join() {
                Ok(Ok(())) => Ok(()),
                Ok(Err(e)) => Err(e),
                Err(e) => Err(NetError::JoinError(e))
            }
        }
        else {
            Err(NetError::NotRunning)
        }
    }

    /// Send data to the server
    pub fn send(&mut self, data: S) -> NetResult<()> {
        // Check the thread is running
        if self.thread_handle.is_none() {
            return Err(NetError::NotRunning)
        }

        self.send_channel.send(ClientCommand::Send(data))?;
        Ok(())
    }

    /// Recieve data from the server.
    ///
    /// Returns a `ServerResponse` enum.
    pub fn recv(&mut self) -> NetResult<ServerResponse<R>> {
        // Check the thread is running
        if self.thread_handle.is_none() {
            return Err(NetError::NotRunning)
        }

        match self.recv_channel.try_recv() {
            Ok(ClientResponse::Received(r)) => Ok(ServerResponse::Received(r)),
            Ok(ClientResponse::ReceivedOutOfOrder(r)) => Ok(ServerResponse::ReceivedOutOfOrder(r)),
            Ok(ClientResponse::Error(e)) => Ok(ServerResponse::Error(e)),
            Ok(ClientResponse::Disconnected) => Ok(ServerResponse::Disconnected),
            Err(e) => match e {
                TryRecvError::Empty => Ok(ServerResponse::None),
                TryRecvError::Disconnected => Err(NetError::ChannelDisconnected)
            }
        }
    }
}

// -----------------------------------------------------------------------------------------------
// PRIVATE FUNCTIONS
// -----------------------------------------------------------------------------------------------

fn client<S, R>(
    mut stream: TcpStream,
    send_rx: Receiver<ClientCommand<S>>,
    recv_tx: Sender<ClientResponse<R>>
) -> NetResult<()>
where
    S: Serialize + Send + 'static,
    R: DeserializeOwned + Send + 'static
{

    // So that we can send and recieve "simultaneously" we set the stream to be non-blocking, 
    // meaning that .send and .recv will return immediately if there's nothing to do.
    stream.set_nonblocking(true)?;

    // Frame counters
    let mut send_frame_counter = 0u64;
    let mut recv_frame_counter = 0u64;

    // Data buffer
    let mut buffer = Vec::with_capacity(1024);

    // Main client handling loop
    loop {
        // Send half
        let server_cmd = send_rx.try_recv();

        match server_cmd {
            // Shutdown command
            Ok(ClientCommand::Shutdown) => {
                // Shutdown the stream and return immediately
                stream.shutdown(Shutdown::Both)?;
                return Ok(())
            },
            // Data to be sent to the 
            Ok(ClientCommand::Send(d)) => {
                // Build the new frame to send
                let frame = Frame {
                    frame_counter: send_frame_counter,
                    data: d
                };

                // Send the frame to the client
                stream.write(frame.to_byte_vec()?.as_slice())?;

                // Increment the send frame counter
                send_frame_counter += 1;
            }
            // The channel is empty nothing to do
            Err(TryRecvError::Empty) => (),
            Err(TryRecvError::Disconnected) => return Err(NetError::ChannelDisconnected)
        }

        // Recieve half
        match stream.read_buffer(&mut buffer) {
            // Client send some data
            Ok(s) => {
                // If there is some data in the buffer attempt to parse it
                if s.len() > 0 {
                    // Parse the data into a frame
                    match Frame::<R>::from_bytes(s) {
                        // If the frame is parsed successfully
                        Ok(frame) => {
                            // If the counters are the same (or current f/c is 0 indicating need to 
                            // init)
                            if recv_frame_counter == 0 
                                || frame.frame_counter == recv_frame_counter
                            {
                                // Set the next expected frame counter
                                recv_frame_counter = frame.frame_counter + 1;
    
                                // Send the received data back to the main thread
                                recv_tx.send(ClientResponse::Received(frame.data))?;
                            }
                            // If the frame counters are out of oreder
                            else {
                                // Set the next expected frame counter
                                recv_frame_counter = frame.frame_counter + 1;
    
                                // Send the received data back to the main thread with the out of order
                                // warning
                                recv_tx.send(ClientResponse::ReceivedOutOfOrder(frame.data))?;
                            }
    
                        },
                        // If the response couldn't be parsed send the error message back to the main
                        // thread
                        Err(e) => {
                            recv_tx.send(ClientResponse::Error(e))?;
                        }
                    }
                }
                
                // Clear the buffer
                buffer.clear();
            },
            // Error recieving
            Err(e) => match e.kind() {
                // If the client hasn't sent some data then WouldBlock is the error kind
                ErrorKind::WouldBlock => (),
                // If the client disconnected send the disconnect message back to the accept loop
                // and return
                ErrorKind::NotConnected 
                    | ErrorKind::ConnectionAborted 
                    | ErrorKind::ConnectionReset
                    => 
                {
                    recv_tx.send(ClientResponse::Disconnected)?;
                    break;
                }
                // Otherwise send the error back to the main thread
                _ => {
                    recv_tx.send(ClientResponse::Error(e.into()))?;
                    break;
                }
            }
        }
    }

    Ok(())
}