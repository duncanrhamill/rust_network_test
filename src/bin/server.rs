// use std::thread;
// use std::net::{TcpListener, TcpStream, Shutdown};
// use std::io::{Read, Write};

use network_test::{Server, ClientResponse};

fn main() -> Result<(), Box<dyn std::error::Error>> {

    let mut server = Server::<u64, u64>::start(4000)?;

    println!("Server started on port 4000");

    loop {
        // Read data from the server
        match server.recv() {
            Ok(ClientResponse::Recieved(r)) => {
                println!("Recieved {:#?}", r);
                println!("Sending response back to client");
                server.send(r)?;
            },
            Ok(ClientResponse::None) => {
                // no data
            },
            Ok(ClientResponse::Disconnected) => {
                println!("Client disconnected");
                break;
            }
            e => {
                println!("Unexpected data from client: {:#?}", e);
            }
        }
    }

    server.shutdown()?;

    Ok(())
}

// type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

// fn main() -> Result<()> {

//     let server = network_test::Server::<LocoTc, LocoTc>::start(4000);

//     let listener = TcpListener::bind("0.0.0.0:4000")?;

//     println!("Open on port 4000");

//     for stream in listener.incoming() {
//         match stream {
//             Ok(stream) => {
//                 println!("New connection from {}", stream.peer_addr()?);
//                 thread::spawn(move || {
//                     handle_client(stream)
//                 });
//             }
//             Err(e) => {
//                 println!("Incoming connection failed: {}", e);
//             }
//         }
//     }

//     Ok(())
// }

// fn handle_client(mut stream: TcpStream) {

//     // Start wiht a 1024 buffer
//     let mut data = vec![0u8; 1024];

//     loop {
//         match stream.read(data.as_mut_slice()) {
//             Ok(0) => {},
//             Ok(size) => {
//                 // Try to read it as a LocoTc
//                 let loco_tc = LocoTc::from_frame_bytes(&data[0..size])
//                     .expect("Failed to deserialize the frame");
//                 println!("Got following data: {:#?}", loco_tc);
//                 // echo back
//                 println!("Recieved {} bytes", size);
//                 stream.write(&data[0..size]).expect("Failed to echo back to client");
//             }
//             Err(_) => {
//                 println!("Error reading from incoming stream");
//                 stream.shutdown(Shutdown::Both).expect("Failed to shutdown stream");
//                 break;
//             }
//         }
//     }
// }