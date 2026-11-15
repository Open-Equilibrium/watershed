//! Spike: prints a thread's projection as JSON. Usage: t3_dump <t3> <home> <project> <thread>
use meta_harness_core::t3::{T3Client, T3Project, T3Server};
use std::path::Path;

#[tokio::main]
async fn main() -> Result<(), String> {
    let args: Vec<String> = std::env::args().collect();
    let [_, binary, home, project, thread] = args.as_slice() else {
        return Err("usage: t3_dump <t3> <home> <project> <thread>".into());
    };
    let server = T3Server::start(Path::new(binary), Path::new(home), Path::new(project)).await?;
    let client = T3Client::connect(&server.origin, server.token()).await?;
    let t3 = T3Project::open(client, project).await?;
    let (_, projection) = t3.projection(thread).await?;
    println!("{projection}");
    server.stop().await;
    Ok(())
}
