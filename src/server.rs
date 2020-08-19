
// -----------------------------------------------------------------------------------------------
// IMPORTS
// -----------------------------------------------------------------------------------------------

use std::{
    thread::{self, JoinHandle}, 
    sync::{
        Arc, 
        mpsc::{channel, Sender, Receiver, TryRecvError}, 
        Mutex,
    }, 
    net::{TcpStream, TcpListener, Shutdown}, 
    io::{self, Write},
};
use serde::{Serialize, de::DeserializeOwned};
use buffer::ReadBuffer;
use super::{Frame, NetResult, NetError};
use io::ErrorKind;

// -----------------------------------------------------------------------------------------------
// STRUCTS
// -----------------------------------------------------------------------------------------------

pub struct Server<S, R>
where
    S: Serialize + Send,
    R: DeserializeOwned + Send
{
    listener_handle: Option<JoinHandle<NetResult<()>>>,
    send_channel: Sender<ServerCommand<S>>,
    recv_channel: Receiver<ServerResponse<R>>
}

// -----------------------------------------------------------------------------------------------
// ENUMS
// -----------------------------------------------------------------------------------------------

#[derive(Debug)]
pub enum ClientResponse<R>
where
    R: DeserializeOwned
{
    None,
    Recieved(R),
    ReceivedOutOfOrder(R),
    Disconnected,
    Error(NetError)
}

enum ServerCommand<S>
where
    S: Serialize + Send
{
    Shutdown,
    Send(S)
}

enum ServerResponse<R>
where
    R: DeserializeOwned + Send
{
    BindSuccessful,
    BindFailed,
    Received(R),
    ReceivedOutOfOrder(R),
    Error(NetError),
    Disconnected
}

// -----------------------------------------------------------------------------------------------
// IMPLS
// -----------------------------------------------------------------------------------------------

impl<S, R> Server<S, R>
where 
    S: Serialize + Send + 'static,
    R: DeserializeOwned + Send + 'static
{
    /// Create a new server and start listening for connections on the given port number.
    pub fn start(port_num: u16) -> NetResult<Self> {

        // Create the channel endpoints
        let (send_tx, send_rx) = channel::<ServerCommand<S>>();
        let (recv_tx, recv_rx) = channel::<ServerResponse<R>>();

        // Clone for the server side send_tx, so that the listener thread can send commands to
        // the client thread such as shutdown.
        let send_tx_clone = send_tx.clone();

        // Start the listener thread
        let listener_handle = thread::spawn(move ||{
            listener(port_num, send_tx_clone, send_rx, recv_tx)
        });

        // Wait for the first message from the listener thread, which will show if the bind was
        // successful or not.
        // TODO: Add timeout?
        match recv_rx.recv() {
            // The bind failed, join the listener thread then retun the error from the thread.
            Ok(ServerResponse::BindFailed) => {
                match listener_handle.join() {
                    Ok(r) => return Err(r.err().unwrap().into()),
                    Err(e) => return Err(NetError::JoinError(e))
                }
            },
            // If there was a disconnect return the error
            Err(e) => return Err(e.into()),
            // Otherwise we can continue on as normal
            _ => ()
        }

        // Create the public endpoint of the server
        Ok(Self {
            listener_handle: Some(listener_handle),
            send_channel: send_tx,
            recv_channel: recv_rx
        })
    }

    /// Send the given data to the client.
    pub fn send(&mut self, data: S) -> NetResult<()> {
        // Check the server is running
        if self.listener_handle.is_none() {
            return Err(NetError::NotRunning)
        }

        self.send_channel.send(ServerCommand::Send(data))?;
        Ok(())
    }

    /// Recieve data from the client.
    ///
    /// This function is non-blocking and will return a `ClientResponse` enum.
    pub fn recv(&mut self) -> NetResult<ClientResponse<R>> {
        // Check the server is running
        if self.listener_handle.is_none() {
            return Err(NetError::NotRunning)
        }

        match self.recv_channel.try_recv() {
            Ok(r) => match r {
                ServerResponse::Disconnected => Ok(ClientResponse::Disconnected),
                ServerResponse::Received(r) => Ok(ClientResponse::Recieved(r)),
                ServerResponse::ReceivedOutOfOrder(r) => Ok(ClientResponse::ReceivedOutOfOrder(r)),
                ServerResponse::Error(e) => Ok(ClientResponse::Error(e)),
                _ => Err(NetError::UnexpectedResponse)
            },
            Err(e) => match e {
                TryRecvError::Empty => Ok(ClientResponse::None),
                TryRecvError::Disconnected => Err(NetError::ChannelDisconnected)
            }
        }
    }

    /// Shutdown the server.
    pub fn shutdown(mut self) -> NetResult<()> {
        // Send the shutdown command to the threads
        self.send_channel.send(ServerCommand::Shutdown)?;

        // Wait for the listener thread to stop
        if let Some(ljh) = self.listener_handle.take() {
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
}

// -----------------------------------------------------------------------------------------------
// PRIVATE FUNCTIONS
// -----------------------------------------------------------------------------------------------

fn listener<S, R>(
    port_num: u16, 
    send_tx: Sender<ServerCommand<S>>,
    send_rx: Receiver<ServerCommand<S>>, 
    recv_tx: Sender<ServerResponse<R>>
) -> NetResult<()>
where
    S: Serialize + Send + 'static,
    R: DeserializeOwned + Send + 'static
{
    // Build the listener and bind
    let listener = TcpListener::bind(("0.0.0.0", port_num));

    // If the listener was succesfully bound then continue to setting up the accept loop
    if let Ok(listener) = listener {
        // Send a message back on the recv_tx indicating success
        recv_tx.send(ServerResponse::BindSuccessful)?;

        // Join handle for client handling thread
        let mut client_jh: Option<JoinHandle<NetResult<()>>> = None;

        // The design here is that there should never be more than one client thread alive at once,
        // therefore spawning off threads in the accept loop is fine. However rust doesn't know 
        // this, instead thinking that we can send the channel endpoints to the new thread because
        // they're moved into the previous thread. Therefore to appease the crab we need to use 
        // some reference counting.
        let arc_send_rx: Arc<Mutex<Receiver<ServerCommand<S>>>> = Arc::new(
            Mutex::new(send_rx)
        );
        let arc_recv_tx: Arc<Mutex<Sender<ServerResponse<R>>>> = Arc::new(
            Mutex::new(recv_tx)
        );

        // Main accept loop
        for stream in listener.incoming() {
            if let Ok(stream) = stream {
                // If there's already a client being handled we should disconnect it and join the
                // handle thread, thereby ensuring the most recent connection is always the one 
                // that's serviced.
                if let Some(cjh) = client_jh.take() {
                    println!("New client connected, shutting down old connection");

                    // Send the shutdown command
                    send_tx.send(ServerCommand::Shutdown)?;

                    // Wait for the join from the client
                    match cjh.join() {
                        Ok(Ok(())) => (),
                        Ok(Err(e)) => return Err(e),
                        Err(e) => return Err(NetError::JoinError(e))
                    }
                }

                // Clone the arcs
                let arc_send_rx_clone = arc_send_rx.clone();
                let arc_recv_tx_clone = arc_recv_tx.clone();

                // At this point no other client thread should be running so we can start a new
                // client thread
                client_jh = Some(thread::spawn(move || {
                    client(stream, arc_send_rx_clone, arc_recv_tx_clone)
                }));
            }
            else {
                // TODO: Log the connection failure and try again
            }
        }

        Ok(())
    }
    // Otherwise, if the listener could not be bound
    else {
        // Send a message back on the recv_tx indicating the failure.
        recv_tx.send(ServerResponse::BindFailed)?;
        Err(NetError::BindError)
    }
}

fn client<S, R>(
    mut stream: TcpStream,
    send_rx: Arc<Mutex<Receiver<ServerCommand<S>>>>,
    recv_tx: Arc<Mutex<Sender<ServerResponse<R>>>>
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
        let server_cmd = send_rx.lock()?.try_recv();

        match server_cmd {
            // Shutdown command
            Ok(ServerCommand::Shutdown) => {
                // Shutdown the stream and return immediately
                stream.shutdown(Shutdown::Both)?;
                return Ok(())
            },
            // Data to be sent to the 
            Ok(ServerCommand::Send(d)) => {
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
                                recv_tx.lock()?.send(ServerResponse::Received(frame.data))?;
                            }
                            // If the frame counters are out of oreder
                            else {
                                // Set the next expected frame counter
                                recv_frame_counter = frame.frame_counter + 1;
    
                                // Send the received data back to the main thread with the out of order
                                // warning
                                recv_tx.lock()?.send(ServerResponse::ReceivedOutOfOrder(frame.data))?;
                            }
    
                        },
                        // If the response couldn't be parsed send the error message back to the main
                        // thread
                        Err(e) => {
                            println!("Couldn't deserialize: {:?}", std::str::from_utf8(s));
                            recv_tx.lock()?.send(ServerResponse::Error(e))?;
                        }
                    }
                }

                // Clear the buffer
                buffer.clear();
            },
            // Error recieving
            Err(e) => {
                match e.kind() {
                    // If the client hasn't sent some data then WouldBlock is the error kind
                    ErrorKind::WouldBlock => (),
                    // If the client disconnected send the disconnect message back to the accept loop
                    // and return
                    ErrorKind::NotConnected 
                    | ErrorKind::ConnectionAborted 
                    | ErrorKind::ConnectionReset
                    => 
                    {
                        println!("Error receiving: {}", e);
                        recv_tx.lock()?.send(ServerResponse::Disconnected)?;
                        break;
                    }
                    // Otherwise send the error back to 
                    _ => {
                        println!("Error receiving: {}", e);
                        return Err(e.into())
                    }
                }
            }
        }
    }

    Ok(())
}