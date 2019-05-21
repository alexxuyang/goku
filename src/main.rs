extern crate futures;
extern crate tokio;
extern crate structopt;

use futures::sync::mpsc::{self, UnboundedSender};
use futures::stream::*;

use tokio::net::TcpListener;
use tokio::prelude::*;
use tokio::io;

use structopt::StructOpt;

use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::collections::HashMap;
use std::iter;
use std::io::BufReader;

#[derive(StructOpt, Debug)]
#[structopt(name = "example")]
struct Opt {
    /// peer's address and port
    #[structopt(short = "p", long = "peers", help = "peer's address to connect")]
    peers: Vec<String>,

    /// peer's address and port
    #[structopt(short = "t", long = "port", help = "port to listen", default_value="8080")]
    port: String,
}

fn main() {
    let opt = Opt::from_args();
    println!("{:?}", opt);

    let addr = format!("127.0.0.1:{}", opt.port);
    let addr = addr.parse().unwrap();
    let listener = TcpListener::bind(&addr).unwrap();

    let connections: Arc<Mutex<HashMap<SocketAddr, UnboundedSender<_>>>> = Arc::new(Mutex::new(HashMap::new()));

    let server = listener.incoming()
        .for_each(move |socket| {
            // get client IP address
            let addr = socket.peer_addr().unwrap();

            // split socket to reader endpoint and writer endpoint
            let (reader, writer) = socket.split();
            let reader = BufReader::new(reader);

            // create channel
            let (tx, rx) = mpsc::unbounded();

            // add peer to connections
            connections.lock().unwrap().insert(addr, tx);

            // clone the connections
            let conn = Arc::clone(&connections);

            // create infinite stream which produce () as value
            let it = stream::iter_ok::<_, std::io::Error>(iter::repeat(()));
            let reader_fut = it
                .fold(reader, move |reader, _| {
                    // read until one line
                    let line = io::read_until(reader, b'\n', Vec::new());

                    // map zero length msg to Err
                    let line = line.and_then(|(reader, msg)| {
                        if msg.len() == 0 {
                            Err(io::Error::new(io::ErrorKind::BrokenPipe, "broken pipe"))
                        } else {
                            Ok((reader, String::from_utf8(msg)))
                        }
                    });

                    // clone the connections used as below task
                    let connections = Arc::clone(&conn);

                    // write msg to all channels
                    line.map(move |(reader, msg)| {
                        let mut map = connections.lock().unwrap();
                        let msg = msg.unwrap();

                        for (key, tx) in map.iter_mut() {
                            if key != &addr {
                                tx.unbounded_send(format!("{}: {}", addr, msg)).unwrap();
                            }
                        }

                        reader
                    })
                });

            // write the data in rx endpoint to socket
            let writer_fut = rx.fold(writer, move |writer, msg| {
                let re = io::write_all(writer, msg);
                let re = re.map(|(writer, _)| writer);
                re.map_err(|_|{})
            });

            let reader_fut = reader_fut.map(|_| {}).map_err(|_| {});
            let writer_fut = writer_fut.map(|_| {}).map_err(|_| {});

            let connections = Arc::clone(&connections);
            let task = reader_fut.select(writer_fut).then(move |_| {
                // remove the connection if either future resolve to the result
                connections.lock().unwrap().remove(&addr);
                println!("{} connection closed!", addr);
                Ok(())
            });

            // start the task
            tokio::spawn(task);

            Ok(())
        })
        .map_err(|_|{});

    // start the server
    tokio::run(server);
}