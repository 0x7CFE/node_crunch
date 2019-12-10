use std::sync::{Arc, Mutex};

use tokio::net::{TcpListener, TcpStream};
use tokio::io::{BufReader, BufWriter, AsyncReadExt, AsyncBufReadExt, AsyncWriteExt};

use log::{info, error, debug};

use crate::error::{NCError};
use crate::nc_node::{NodeMessage};

#[derive(Debug, Clone)]
pub enum ServerMessage {
    ServerHasData(Vec<u8>),
    ServerFinished,
}

pub trait NC_Server {
    fn finished(&self) -> bool;
    fn prepare_data_for_node(&mut self) -> Vec<u8>;
    fn process_data_from_node(&mut self, data: &Vec<u8>);
}

async fn start_server<T: 'static + NC_Server + Send>(server: T) -> Result<(), NCError> {

    let addr = "127.0.0.1:9000".to_string();
    let mut socket = TcpListener::bind(&addr).await.map_err(|e| NCError::TcpBind(e))?;
    debug!("Listening on: {}", addr);

    let quit = Arc::new(Mutex::new(false));
    let server = Arc::new(Mutex::new(server));

    while !(*quit.lock().map_err(|_| NCError::QuitLock)?) {
        let (stream, node) = socket.accept().await.map_err(|e| NCError::SocketAccept(e))?;
        debug!("Connection from: {}", node.to_string());
        tokio::spawn(handle_node(server.clone(), stream, quit.clone()));
    }

    Ok(())
}

async fn handle_node<T: NC_Server + Send>(server: Arc<Mutex<T>>, mut stream: TcpStream, quit: Arc<Mutex<bool>>) -> Result<(), NCError> {
    let (reader, writer) = stream.split();
    let mut buf_reader = BufReader::new(reader);
    
    let message_length: u64 = buf_reader.read_u64().await.map_err(|e| NCError::ReadU64(e))?;
    let mut buffer = vec![0; message_length as usize];
    let num_of_bytes_read: usize = buf_reader.read(&mut buffer[..]).await.map_err(|e| NCError::ReadBuffer(e))?;

    debug!("Message length: {}, number of bytes read: {}", message_length, num_of_bytes_read);

    match decode(buffer)? {
        NodeMessage::NodeNeedsData => {

        }
        NodeMessage::NodeHasData(new_data) => {
            let mut server = server.lock().map_err(|_| NCError::ServerLock)?;
            server.process_data_from_node(&new_data);
            if server.finished() {
                let mut quit = quit.lock().map_err(|_| NCError::QuitLock)?;
                *quit = true;
            }
        }
    }

    Ok(())
}

fn decode(buffer: Vec<u8>) -> Result<NodeMessage, NCError> {
    Ok(NodeMessage::NodeNeedsData)
}