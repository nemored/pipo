use pipo;

#[tokio::main]
async fn main() {
    match pipo::inner_main().await {
        Ok(_) => (),
        Err(e) => {
            eprintln!("{:#}", e);
            ()
        }
    }
}
