use network_test::{Client, ServerResponse};

fn main() -> Result<(), Box<dyn std::error::Error>> {

    // Connect the client to the server
    let mut client = Client::<u64, u64>::connect(4000)?;

    println!("Connected to client, sending dummy data");

    let mut data = 0;
    let mut run = true;

    while run {
        // Send data to the server
        client.send(data)?;

        loop {
            // Wait for the response from the server
            match client.recv()? {
                ServerResponse::Received(r) => {
                    println!("Recieved: {:#?}", r);
                    data += 1;
                    break;
                },
                ServerResponse::Disconnected => {
                    println!("Disconnected");
                    run = false;
                    break;
                },
                ServerResponse::None => (),
                e => println!("Unexpected response from the server: {:#?}", e)
            }
        }

        std::thread::sleep(std::time::Duration::from_secs_f64(0.25));
    }

    client.shutdown()?;

    Ok(())
}