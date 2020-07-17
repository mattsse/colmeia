use async_std::{net::TcpStream, sync::RwLock};
use std::{net::SocketAddr, sync::Arc};

use colmeia_hypercore::*;
use colmeia_hypercore_utils::{parse, UrlResolution};

fn name() -> String {
    let args: Vec<String> = std::env::args().skip(1).collect();
    args.first().expect("must have dat name as argument").into()
}

fn address() -> SocketAddr {
    let args: Vec<String> = std::env::args().skip(2).collect();
    let input = args
        .first()
        .expect("must have dat server:port name as argument");
    input.parse().expect("invalid ip:port as input")
}

// TODO: send to a folder
// use std::path::PathBuf;
// fn folder() -> PathBuf {
//     let args: Vec<String> = std::env::args().skip(3).collect();
//     args.first().expect("must have folder as argument").into()
// }

fn main() {
    env_logger::init();

    let key = name();
    let address = address();

    // let path = name();
    async_std::task::block_on(async {
        let tcp_stream = TcpStream::connect(address)
            .await
            .expect("could not open address");
        let hash = parse(&key).expect("invalid dat argument");

        let hash = match hash {
            UrlResolution::HashUrl(result) => result,
            _ => panic!("invalid hash key"),
        };

        let client = hypercore_protocol::ProtocolBuilder::initiator().connect(tcp_stream);

        let hyperdrive = colmeia_hypercore::in_memmory(*hash.public_key())
            .await
            .expect("Invalid intialization");
        let hyperdrive = Arc::new(RwLock::new(hyperdrive));
        sync_hyperdrive(client, hyperdrive).await;
    });
}
