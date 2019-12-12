use std::sync::{Arc, Mutex};
use std::error;
use std::net::{IpAddr, SocketAddr};

use tokio::net::{TcpListener, TcpStream};
use tokio::io::{BufReader, BufWriter};
use tokio::task;

use log::{error, debug};

use serde::{Serialize, Deserialize};

use crate::nc_error::{NC_Error};
use crate::nc_node::{NC_NodeMessage};
use crate::nc_util::{nc_send_message, nc_receive_message, nc_encode_data, nc_decode_data};
use crate::nc_config::{NC_Configuration};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum NC_ServerMessage {
    ServerHasData(Vec<u8>),
    ServerFinished,
    // ServerHeartBeatOK,
}

pub trait NC_Server {
    fn prepare_data_for_node(&mut self, node_id: u128) -> Result<Vec<u8>, Box<dyn error::Error + Send>>;
    fn process_data_from_node(&mut self, node_id: u128, data: &Vec<u8>) -> Result<bool, Box<dyn error::Error + Send>>;
}

pub async fn start_server<T: 'static + NC_Server + Send>(nc_server: T, config: NC_Configuration) -> Result<(), NC_Error> {
    let addr = SocketAddr::new("0.0.0.0".parse().unwrap(), config.port);
    let mut socket = TcpListener::bind(addr).await.map_err(|e| NC_Error::TcpBind(e))?;

    debug!("Listening on: {}", addr);

    let quit = Arc::new(Mutex::new(false));
    let nc_server = Arc::new(Mutex::new(nc_server));

    while !(*quit.lock().map_err(|_| NC_Error::QuitLock)?) {
        let (stream, node) = socket.accept().await.map_err(|e| NC_Error::SocketAccept(e))?;
        let nc_server = nc_server.clone();
        let quit = quit.clone();

        debug!("Connection from: {}", node.to_string());

        tokio::spawn(async move {
            match handle_node(nc_server, stream, quit).await {
                Ok(_) => debug!("handle node finished"),
                Err(e) => error!("handle node returned an error: {}", e),
            }
        });
    }

    Ok(())
}

async fn handle_node<T: NC_Server>(nc_server: Arc<Mutex<T>>, mut stream: TcpStream, quit: Arc<Mutex<bool>>) -> Result<(), NC_Error> {
    let (reader, writer) = stream.split();
    let mut buf_reader = BufReader::new(reader);
    let mut buf_writer = BufWriter::new(writer);
    
    debug!("Receiving message from node");
    let (num_of_bytes_read, buffer) = nc_receive_message(&mut buf_reader).await?;

    debug!("handle_node: number of bytes read: {}", num_of_bytes_read);
    debug!("Decoding message");
    match nc_decode_data(&buffer)? {
        NC_NodeMessage::NodeNeedsData(node_id) => {
            let quit = *quit.lock().map_err(|_| NC_Error::QuitLock)?;
            if quit {
                debug!("Encoding message ServerFinished");
                let message = nc_encode_data(&NC_ServerMessage::ServerFinished)?;

                debug!("Sending message to node");
                nc_send_message(&mut buf_writer, message).await?;

                debug!("No more data for node, server has finished");
            } else {
                let new_data = {
                    let mut nc_server = nc_server.lock().map_err(|_| NC_Error::ServerLock)?;

                    debug!("Prepare new data for node");
                    task::block_in_place(move || {
                        nc_server.prepare_data_for_node(node_id).map_err(|e| NC_Error::ServerPrepare(e))
                    })?
                }; // Mutex for nc_server needs to be dropped here

                debug!("Encoding message ServerHasData");
                let message = nc_encode_data(&NC_ServerMessage::ServerHasData(new_data))?;
                let message_length = message.len() as u64;

                debug!("Sending message to node");
                nc_send_message(&mut buf_writer, message).await?;
    
                debug!("New data sent to node, message_length: {}", message_length);
            }
        }
        NC_NodeMessage::NodeHasData((node_id, new_data)) => {
            debug!("New processed data received from node: {}", node_id);
            let finished = {
                let mut nc_server = nc_server.lock().map_err(|_| NC_Error::ServerLock)?;

                debug!("Processing data from node: {}", node_id);
                task::block_in_place(move || {
                    nc_server.process_data_from_node(node_id, &new_data)
                        .map_err(|e| NC_Error::ServerProcess(e))
                })?
            }; // Mutex for nc_server needs to be dropped here

            if finished {
                debug!("Job is finished!");
                {
                    let mut quit = quit.lock().map_err(|_| NC_Error::QuitLock)?;
                    *quit = true;
                } // Mutex for quit needs to be dropped here

                debug!("Encoding message ServerFinished");
                let message = nc_encode_data(&NC_ServerMessage::ServerFinished)?;

                debug!("Sending message to node: {}", node_id);
                nc_send_message(&mut buf_writer, message).await?;
            }
        }
    }

    Ok(())
}
